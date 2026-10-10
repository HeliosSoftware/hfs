//! Shared SQL runner preparation: validate, resolve groups, bind, then emit.
//!
//! Runtime filters are attached to the IR, so a caller's quoted SQL-looking
//! literal cannot affect their placement. Backend loaders only fetch Group
//! documents; interpretation and merging of membership live here.

use std::collections::HashSet;
use std::future::Future;

use helios_fhir::FhirVersion;
use serde_json::Value;

use crate::core::sof_runner::{SofError, ViewFilters};

use super::compile_view::build_plan;
use super::compiler::{CompileTarget, CompiledQuery, OutputLimitStrategy, SqlDialect, dialect_for};
use super::emit::emit_plan;
use super::ir::{CompartmentFilter, LitValue, PlanNode, ResourceFilter};

/// Neutral values in SQL slot order. Backend adapters retain their native
/// representations, especially PostgreSQL timestamps and SQLite numeric constants.
#[derive(Debug, Clone)]
pub(super) enum RuntimeParam {
    Text(String),
    Literal(LitValue),
    TextList(Vec<String>),
    Timestamp(chrono::DateTime<chrono::Utc>),
}

/// Everything a producer needs after the shared preparation phase.
pub(super) struct PreparedSqlRun {
    pub query: CompiledQuery,
    pub params: Vec<RuntimeParam>,
    pub client_limit: Option<usize>,
}

/// A validated view retained as IR until group membership has been resolved.
pub(super) struct SqlRunPlan {
    plan: PlanNode,
    constants: Vec<LitValue>,
    resource_type: String,
    dialect: SqlDialect,
    fhir_version: FhirVersion,
    limit_strategy: OutputLimitStrategy,
}

impl SqlRunPlan {
    pub(super) fn compile(
        view: &Value,
        dialect: SqlDialect,
        fhir_version: FhirVersion,
    ) -> Result<Self, SofError> {
        let target = match dialect {
            SqlDialect::Sqlite => CompileTarget::Sqlite,
            SqlDialect::Postgres => CompileTarget::Postgres,
        };
        let dial = dialect_for(dialect);
        let (plan, constants) = build_plan(view, dial.as_ref(), target, fhir_version)?;
        // Emission can reject a computed source (#1862). Check before any
        // group I/O and before an empty-group result could bypass that error.
        emit_plan(&plan, dial.as_ref())?;
        let resource_type = view["resource"]
            .as_str()
            .expect("build_plan checked ViewDefinition.resource")
            .to_string();
        let limit_strategy = OutputLimitStrategy::for_plan(&plan);
        Ok(Self {
            plan,
            constants,
            resource_type,
            dialect,
            fhir_version,
            limit_strategy,
        })
    }

    /// Finish a view whose group references have already been folded into
    /// `patient`. The allocator follows consumed constant bindings, not the
    /// number of declarations or the emitter's expression-local counter.
    pub(super) fn finish(
        mut self,
        tenant_id: &str,
        filters: &ViewFilters,
    ) -> Result<PreparedSqlRun, SofError> {
        debug_assert!(filters.group.is_empty(), "resolve groups before binding");
        let mut params = vec![
            RuntimeParam::Text(tenant_id.to_string()),
            RuntimeParam::Text(self.resource_type.clone()),
        ];
        params.extend(self.constants.iter().cloned().map(RuntimeParam::Literal));
        let filter =
            bind_resource_filter(self.fhir_version, &self.resource_type, filters, &mut params);
        attach_resource_filter(&mut self.plan, &filter);
        let emitted = emit_plan(&self.plan, dialect_for(self.dialect).as_ref())?;
        let mut query = CompiledQuery {
            sql: emitted.sql,
            columns: emitted.columns,
            column_decodes: emitted.column_decodes,
            constants: self.constants,
        };
        let (sql_limit, client_limit) =
            output_limits(self.dialect, self.limit_strategy, filters.limit);
        if let Some(limit) = sql_limit {
            query.sql.push_str(&format!("\nLIMIT {limit}"));
        }
        Ok(PreparedSqlRun {
            query,
            params,
            client_limit,
        })
    }
}

/// One run preamble for both SQL backends. `None` is the existing empty-group
/// short-circuit, reached only after full compilation and emission validation.
pub(super) async fn prepare_sql_run<F, Fut>(
    view: &Value,
    dialect: SqlDialect,
    fhir_version: FhirVersion,
    tenant_id: &str,
    mut filters: ViewFilters,
    load_groups: F,
) -> Result<Option<PreparedSqlRun>, SofError>
where
    F: FnOnce(Vec<String>) -> Fut,
    Fut: Future<Output = Result<Vec<Value>, SofError>>,
{
    let plan = SqlRunPlan::compile(view, dialect, fhir_version)?;
    if !filters.group.is_empty() {
        let documents = load_groups(filters.group.clone()).await?;
        if !resolve_group_filter(&mut filters, &documents) {
            return Ok(None);
        }
    }
    plan.finish(tenant_id, &filters).map(Some)
}

/// Resolve only Patient member references, preserving bare/unknown Group
/// handling and the union with explicit patients. Sorting references makes
/// bindings deterministic; it does not sort result rows or collection cells.
fn resolve_group_filter(filters: &mut ViewFilters, groups: &[Value]) -> bool {
    let resolved = helios_sof::resolve_group_members_to_patient_refs(&filters.group, groups);
    if resolved.is_empty() && filters.patient.is_empty() {
        return false;
    }
    let mut resolved: Vec<_> = resolved.into_iter().collect();
    resolved.sort();
    let mut existing: HashSet<String> = filters.patient.iter().cloned().collect();
    for patient in resolved {
        if existing.insert(patient.clone()) {
            filters.patient.push(patient);
        }
    }
    filters.group.clear();
    true
}

fn push_param(params: &mut Vec<RuntimeParam>, value: RuntimeParam) -> usize {
    params.push(value);
    params.len()
}

fn bind_resource_filter(
    fhir_version: FhirVersion,
    resource_type: &str,
    filters: &ViewFilters,
    params: &mut Vec<RuntimeParam>,
) -> ResourceFilter {
    let since = filters
        .since
        .map(|since| push_param(params, RuntimeParam::Timestamp(since)));
    let compartment = if filters.patient.is_empty() {
        None
    } else if resource_type == "Patient" {
        let ids = filters
            .patient
            .iter()
            .map(|reference| {
                reference
                    .strip_prefix("Patient/")
                    .unwrap_or(reference)
                    .to_string()
            })
            .collect();
        Some(CompartmentFilter::Owner {
            refs: push_param(params, RuntimeParam::TextList(ids)),
        })
    } else {
        let names = helios_fhir::compartment_params(fhir_version, "Patient", resource_type);
        if names.is_empty() {
            Some(CompartmentFilter::NoMatches)
        } else {
            let param_names = names
                .iter()
                .map(|name| push_param(params, RuntimeParam::Text((*name).to_string())))
                .collect();
            let references = filters
                .patient
                .iter()
                .map(|reference| {
                    if reference.starts_with("Patient/") {
                        reference.clone()
                    } else {
                        format!("Patient/{reference}")
                    }
                })
                .collect();
            Some(CompartmentFilter::SearchIndex {
                param_names,
                refs: push_param(params, RuntimeParam::TextList(references)),
            })
        }
    };
    ResourceFilter { since, compartment }
}

fn attach_resource_filter(plan: &mut PlanNode, filter: &ResourceFilter) {
    match plan {
        PlanNode::Scan {
            filter: scan_filter,
            ..
        } => *scan_filter = filter.clone(),
        PlanNode::Project { parent, .. }
        | PlanNode::Filter { parent, .. }
        | PlanNode::LateralUnnest { parent, .. }
        | PlanNode::Recurse { parent, .. } => attach_resource_filter(parent, filter),
        PlanNode::Union(branches) => {
            for branch in branches {
                attach_resource_filter(branch, filter);
            }
        }
    }
}

/// A representable SQL cap is sufficient for fully ordered flat/forEach rows.
/// SQLite keeps its legacy SQL plus client cap for unions and recursion;
/// PostgreSQL keeps their client cap. Oversized caps also stay on the client.
fn output_limits(
    dialect: SqlDialect,
    strategy: OutputLimitStrategy,
    limit: Option<usize>,
) -> (Option<i64>, Option<usize>) {
    let sql_limit = if dialect == SqlDialect::Sqlite || strategy == OutputLimitStrategy::Direct {
        limit.and_then(|limit| i64::try_from(limit).ok())
    } else {
        None
    };
    let client_limit = if strategy == OutputLimitStrategy::Direct && sql_limit.is_some() {
        None
    } else {
        limit
    };
    (sql_limit, client_limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn flat_view() -> Value {
        json!({"resource":"Patient", "select":[{"column":[{"name":"id","path":"id"}]}]})
    }

    #[test]
    fn caps_use_sql_only_for_complete_orders_and_keep_legacy_fallbacks() {
        for dialect in [SqlDialect::Sqlite, SqlDialect::Postgres] {
            for limit in [0, 1, 50, 10_000] {
                assert_eq!(
                    output_limits(dialect, OutputLimitStrategy::Direct, Some(limit)),
                    (Some(limit as i64), None)
                );
                let legacy_sql = (dialect == SqlDialect::Sqlite).then_some(limit as i64);
                assert_eq!(
                    output_limits(dialect, OutputLimitStrategy::RuntimeOnly, Some(limit)),
                    (legacy_sql, Some(limit))
                );
            }
            for strategy in [
                OutputLimitStrategy::Direct,
                OutputLimitStrategy::RuntimeOnly,
            ] {
                assert_eq!(output_limits(dialect, strategy, None), (None, None));
                #[cfg(target_pointer_width = "64")]
                for limit in [i64::MAX as usize + 1, usize::MAX] {
                    assert_eq!(
                        output_limits(dialect, strategy, Some(limit)),
                        (None, Some(limit))
                    );
                }
            }
            #[cfg(target_pointer_width = "64")]
            assert_eq!(
                output_limits(
                    dialect,
                    OutputLimitStrategy::Direct,
                    Some(i64::MAX as usize)
                ),
                (Some(i64::MAX), None)
            );
        }
    }

    #[test]
    fn runtime_slots_follow_consumed_constants_and_are_shared_by_union_branches() {
        let view = json!({"resource":"Patient", "constant":[
            {"name":"unused","valueString":"ignored"},
            {"name":"second","valueString":"B"},
            {"name":"first","valueString":"A"}],
            "select":[{"unionAll":[
                {"column":[{"name":"value","path":"%first"}]},
                {"column":[{"name":"value","path":"%second"}]},
                {"column":[{"name":"value","path":"%first"}]}]}]});
        let filters = ViewFilters {
            since: Some("2024-01-01T00:00:00Z".parse().unwrap()),
            patient: vec!["Patient/p1".into()],
            ..Default::default()
        };
        for dialect in [SqlDialect::Sqlite, SqlDialect::Postgres] {
            let run = SqlRunPlan::compile(&view, dialect, FhirVersion::default_enabled())
                .unwrap()
                .finish("tenant", &filters)
                .unwrap();
            assert_eq!(run.query.constants.len(), 2);
            assert!(matches!(&run.params[2], RuntimeParam::Literal(LitValue::Str(s)) if s == "A"));
            assert!(matches!(&run.params[3], RuntimeParam::Literal(LitValue::Str(s)) if s == "B"));
            assert!(matches!(&run.params[4], RuntimeParam::Timestamp(_)));
            assert!(matches!(&run.params[5], RuntimeParam::TextList(refs) if refs == &["p1"]));
            let placeholder = dialect_for(dialect).placeholder(5);
            assert_eq!(
                run.query
                    .sql
                    .matches(&format!("r.last_updated >= {placeholder}"))
                    .count(),
                3
            );
            assert_eq!(run.params.len(), 6, "branches reuse every runtime slot");
        }
    }

    #[test]
    fn constant_and_runtime_slots_are_reused_by_every_seed_and_resource_rejoin() {
        let view = json!({"resource":"QuestionnaireResponse",
        "constant":[{"name":"label","valueString":"kept"}],
        "select":[
            {"column":[{"name":"resource_id","path":"id"},{"name":"label","path":"%label"}]},
            {"repeat":["item","answer.item"],"column":[{"name":"link_id","path":"linkId"}]}
        ]});
        let filters = ViewFilters {
            since: Some("2024-01-01T00:00:00Z".parse().unwrap()),
            patient: vec!["Patient/p1".into()],
            ..Default::default()
        };
        let version = FhirVersion::default_enabled();
        let name_count =
            helios_fhir::compartment_params(version, "Patient", "QuestionnaireResponse").len();
        assert!(name_count > 0, "fixture belongs to the Patient compartment");
        for dialect in [SqlDialect::Sqlite, SqlDialect::Postgres] {
            let run = SqlRunPlan::compile(&view, dialect, version)
                .unwrap()
                .finish("tenant", &filters)
                .unwrap();
            let dial = dialect_for(dialect);
            let since = format!("r.last_updated >= {}", dial.placeholder(4));
            assert_eq!(
                run.query.sql.matches(&since).count(),
                3,
                "two seeds and one rejoin"
            );
            assert_eq!(
                run.query
                    .sql
                    .matches("EXISTS (SELECT 1 FROM search_index si")
                    .count(),
                3
            );
            assert_eq!(run.params.len(), 5 + name_count);
            assert!(
                matches!(&run.params[2], RuntimeParam::Literal(LitValue::Str(value)) if value == "kept")
            );
            assert!(matches!(&run.params[3], RuntimeParam::Timestamp(_)));
            assert!(
                matches!(run.params.last(), Some(RuntimeParam::TextList(refs)) if refs == &["Patient/p1"])
            );
        }
    }

    #[test]
    fn sql_looking_literals_do_not_change_filter_placement() {
        for dialect in [SqlDialect::Sqlite, SqlDialect::Postgres] {
            let dial = dialect_for(dialect);
            let anchor_literal = format!(
                "r.tenant_id = {}\n  AND r.resource_type = {}\n  AND r.is_deleted = {}",
                dial.placeholder(1),
                dial.placeholder(2),
                dial.bool_false()
            );
            let mut view = flat_view();
            view["select"][0]["column"] = json!([
                {"name":"scan_literal","path":"'resources r'"},
                {"name":"anchor_literal","path":format!("'{anchor_literal}'")}
            ]);
            let filters = ViewFilters {
                since: Some("2024-01-01T00:00:00Z".parse().unwrap()),
                patient: vec!["Patient/p1".into()],
                ..Default::default()
            };
            let run = SqlRunPlan::compile(&view, dialect, FhirVersion::default_enabled())
                .unwrap()
                .finish("tenant", &filters)
                .unwrap();
            assert!(
                run.query
                    .sql
                    .contains(&dial.string_literal(&anchor_literal))
            );
            assert!(run.query.sql.contains("'resources r'"));
            assert_eq!(run.query.sql.matches("r.last_updated >=").count(), 1);
            assert_eq!(run.params.len(), 4);
        }
    }

    #[tokio::test]
    async fn invalid_emission_is_checked_before_empty_group_or_loader() {
        let view = json!({"resource":"Patient", "where":[{"path":"extension('a\\u0000b').where(true).exists()"}],
            "select":[{"column":[{"name":"id","path":"id"}]}]});
        for dialect in [SqlDialect::Sqlite, SqlDialect::Postgres] {
            let result = prepare_sql_run(
                &view,
                dialect,
                FhirVersion::default_enabled(),
                "tenant",
                ViewFilters {
                    group: vec!["Group/empty".into()],
                    ..Default::default()
                },
                |_| async { panic!("invalid emission must fail before group lookup") },
            )
            .await;
            assert!(
                matches!(result, Err(SofError::Uncompilable { reason }) if reason.contains("NUL"))
            );
        }
    }

    #[tokio::test]
    async fn groups_merge_once_with_explicit_patients_and_bind_stably() {
        let filters = ViewFilters {
            patient: vec!["Patient/p1".into()],
            group: vec!["bare".into(), "Group/other".into(), "Group/missing".into()],
            ..Default::default()
        };
        let run = prepare_sql_run(&flat_view(), SqlDialect::Sqlite,
            FhirVersion::default_enabled(), "tenant", filters,
            |refs| async move {
                assert_eq!(refs, ["bare", "Group/other", "Group/missing"]);
                Ok(vec![
                    json!({"resourceType":"Group","id":"bare","member":[
                        {"entity":{"reference":"Patient/p3"}}, {"entity":{"reference":"Patient/p1"}},
                        {"entity":{"reference":"Device/d1"}}]}),
                    json!({"resourceType":"Group","id":"other","member":[
                        {"entity":{"reference":"Patient/p2"}}, {"entity":{"reference":"Patient/p3"}}]})])
            }).await.unwrap().unwrap();
        assert!(
            matches!(&run.params[2], RuntimeParam::TextList(refs) if refs == &["p1","p2","p3"])
        );
    }

    #[tokio::test]
    async fn empty_group_is_empty_unless_explicit_patients_exist() {
        for patient in [vec![], vec!["Patient/p1".into()]] {
            let run = prepare_sql_run(
                &flat_view(),
                SqlDialect::Postgres,
                FhirVersion::default_enabled(),
                "tenant",
                ViewFilters {
                    patient: patient.clone(),
                    group: vec!["Group/missing".into()],
                    ..Default::default()
                },
                |_| async { Ok(vec![]) },
            )
            .await
            .unwrap();
            assert_eq!(run.is_none(), patient.is_empty());
        }
    }
}
