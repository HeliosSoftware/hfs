//! Chain Query Builder for FHIR Search (PostgreSQL backend).
//!
//! Generates efficient SQL subqueries for:
//! - Forward chained parameters (e.g., `Observation?subject.organization.name=Hospital`)
//! - Reverse chained parameters (`_has`) (e.g., `Patient?_has:Observation:subject:code=1234-5`)
//!
//! Uses the `search_index` table to resolve chains via SQL subqueries instead
//! of in-memory iteration. Mirrors the SQLite implementation in
//! `crates/persistence/src/backends/sqlite/search/chain_builder.rs` with
//! Postgres syntax adaptations: `$N` placeholders, `ILIKE`, `POSITION(... in ...)`
//! for substring index, and `LIKE ESCAPE '\'`.

use std::sync::Arc;

use parking_lot::RwLock;

use crate::error::{BackendError, StorageResult};
use crate::search::SearchParameterRegistry;
use crate::types::{ChainConfig, ReverseChainedParameter, SearchParamType, SearchValue};

use super::query_builder::{SqlFragment, SqlParam};

/// A single link in a forward chain.
#[derive(Debug, Clone)]
pub struct ChainLink {
    /// Reference parameter being chained through.
    pub reference_param: String,
    /// Target resource type resolved from the registry or explicit modifier.
    pub target_type: String,
}

/// A parsed forward chain with resolved types.
#[derive(Debug, Clone)]
pub struct ParsedChain {
    /// Chain links from base to target.
    pub links: Vec<ChainLink>,
    /// Terminal parameter name to search on.
    pub terminal_param: String,
    /// Search parameter type of the terminal parameter.
    pub terminal_type: SearchParamType,
}

/// Errors specific to chain parsing.
#[derive(Debug, Clone)]
pub enum ChainError {
    /// Chain exceeds maximum allowed depth.
    MaxDepthExceeded {
        /// Depth of the chain that was rejected.
        depth: usize,
        /// Configured maximum forward-chain depth.
        max: usize,
    },
    /// Reference parameter not found in registry.
    UnknownReferenceParam {
        /// Resource type the reference parameter was looked up against.
        resource_type: String,
        /// Reference parameter name.
        param: String,
    },
    /// Terminal parameter not found.
    UnknownTerminalParam {
        /// Resource type the terminal parameter was looked up against.
        resource_type: String,
        /// Terminal parameter name.
        param: String,
    },
    /// Chain is empty.
    EmptyChain,
    /// Invalid chain syntax.
    InvalidSyntax {
        /// Human-readable parser failure detail.
        message: String,
    },
}

impl std::fmt::Display for ChainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChainError::MaxDepthExceeded { depth, max } => {
                write!(
                    f,
                    "Chain depth {} exceeds maximum allowed depth {}",
                    depth, max
                )
            }
            ChainError::UnknownReferenceParam {
                resource_type,
                param,
            } => write!(
                f,
                "Unknown reference parameter '{}' for resource type '{}'",
                param, resource_type
            ),
            ChainError::UnknownTerminalParam {
                resource_type,
                param,
            } => write!(
                f,
                "Unknown terminal parameter '{}' for resource type '{}'",
                param, resource_type
            ),
            ChainError::EmptyChain => write!(f, "Empty chain"),
            ChainError::InvalidSyntax { message } => write!(f, "Invalid chain syntax: {}", message),
        }
    }
}

impl From<ChainError> for BackendError {
    fn from(e: ChainError) -> Self {
        BackendError::Internal {
            backend_name: "postgres".to_string(),
            message: e.to_string(),
            source: None,
        }
    }
}

/// Builder for chain SQL queries.
pub struct ChainQueryBuilder {
    #[allow(dead_code)]
    tenant_id: String,
    base_type: String,
    registry: Arc<RwLock<SearchParameterRegistry>>,
    config: ChainConfig,
    /// Parameter offset for `$N` placeholders.
    ///
    /// Callers typically reserve `$1` for `tenant_id`, so the default offset
    /// of `1` makes the first chain-supplied param `$2`.
    param_offset: usize,
}

impl ChainQueryBuilder {
    /// Creates a new chain query builder rooted at `base_type` in the given tenant.
    pub fn new(
        tenant_id: impl Into<String>,
        base_type: impl Into<String>,
        registry: Arc<RwLock<SearchParameterRegistry>>,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            base_type: base_type.into(),
            registry,
            config: ChainConfig::default(),
            param_offset: 1,
        }
    }

    /// Sets the chain depth configuration.
    pub fn with_config(mut self, config: ChainConfig) -> Self {
        self.config = config;
        self
    }

    /// Sets the parameter offset used when allocating `$N` placeholders.
    pub fn with_param_offset(mut self, offset: usize) -> Self {
        self.param_offset = offset;
        self
    }

    /// Parses a chain string (e.g., `"subject.organization.name"`) into
    /// resolved `ChainLink`s plus the terminal parameter.
    pub fn parse_chain(&self, chain_str: &str) -> Result<ParsedChain, ChainError> {
        if chain_str.is_empty() {
            return Err(ChainError::EmptyChain);
        }

        let parts: Vec<&str> = chain_str.split('.').collect();
        if parts.len() < 2 {
            return Err(ChainError::InvalidSyntax {
                message: "Chain must have at least two parts (reference.param)".to_string(),
            });
        }

        let chain_depth = parts.len() - 1;
        if !self.config.validate_forward_depth(chain_depth) {
            return Err(ChainError::MaxDepthExceeded {
                depth: chain_depth,
                max: self.config.max_forward_depth,
            });
        }

        let mut links = Vec::new();
        let mut current_type = self.base_type.clone();

        for part in parts.iter().take(parts.len() - 1) {
            let (ref_param, explicit_type) = parse_chain_part(part);
            let target_type = self.resolve_target_type(&current_type, &ref_param, explicit_type)?;
            links.push(ChainLink {
                reference_param: ref_param,
                target_type: target_type.clone(),
            });
            current_type = target_type;
        }

        let terminal_param = parts[parts.len() - 1].to_string();
        let terminal_type = self.resolve_terminal_type(&current_type, &terminal_param)?;

        Ok(ParsedChain {
            links,
            terminal_param,
            terminal_type,
        })
    }

    fn resolve_target_type(
        &self,
        resource_type: &str,
        ref_param: &str,
        explicit_type: Option<String>,
    ) -> Result<String, ChainError> {
        if let Some(t) = explicit_type {
            return Ok(t);
        }

        let registry = self.registry.read();
        if let Some(param_def) = registry.get_param(resource_type, ref_param) {
            if param_def.param_type != SearchParamType::Reference {
                return Err(ChainError::UnknownReferenceParam {
                    resource_type: resource_type.to_string(),
                    param: ref_param.to_string(),
                });
            }
            if let Some(ref targets) = param_def.target {
                if targets.len() == 1 {
                    return Ok(targets[0].clone());
                }
                // Empty or multiple targets — fall through to inference,
                // matching SQLite's behavior so chained queries against
                // ambiguous references (e.g. `subject` -> Patient|Group|...)
                // pick the same default both backends agree on.
            }
        }

        Ok(crate::search::chain_resolver::infer_target_type(ref_param))
    }

    fn resolve_terminal_type(
        &self,
        resource_type: &str,
        param_name: &str,
    ) -> Result<SearchParamType, ChainError> {
        let registry = self.registry.read();
        if let Some(param_def) = registry.get_param(resource_type, param_name) {
            return Ok(param_def.param_type);
        }
        // Last-resort heuristic for params not in the registry, matching SQLite.
        match param_name {
            "_id" | "id" => Ok(SearchParamType::Token),
            "_lastUpdated" => Ok(SearchParamType::Date),
            "name" | "family" | "given" | "text" | "display" => Ok(SearchParamType::String),
            "identifier" | "code" | "status" | "type" | "category" => Ok(SearchParamType::Token),
            _ => Err(ChainError::UnknownTerminalParam {
                resource_type: resource_type.to_string(),
                param: param_name.to_string(),
            }),
        }
    }

    /// Builds SQL for a forward chain query as nested subqueries.
    ///
    /// For `Observation?subject.organization.name=Hospital` (assuming
    /// `param_offset = 1`, so `$1` is `tenant_id`):
    ///
    /// ```sql
    /// r.id IN (
    ///   SELECT si1.resource_id FROM search_index si1
    ///   WHERE si1.tenant_id = $1 AND si1.resource_type = 'Observation'
    ///     AND si1.param_name = 'subject'
    ///     AND si1.value_reference IN (
    ///       SELECT 'Patient/' || si2.resource_id FROM search_index si2
    ///       WHERE si2.tenant_id = $1 AND si2.resource_type = 'Patient'
    ///         AND si2.param_name = 'organization'
    ///         AND si2.value_reference IN (
    ///           SELECT 'Organization/' || si3.resource_id FROM search_index si3
    ///           WHERE si3.tenant_id = $1 AND si3.resource_type = 'Organization'
    ///             AND si3.param_name = 'name'
    ///             AND si3.value_string ILIKE $2 ESCAPE '\'
    ///         )
    ///     )
    /// )
    /// ```
    pub fn build_forward_chain_sql(
        &self,
        chain: &ParsedChain,
        value: &SearchValue,
    ) -> StorageResult<SqlFragment> {
        if chain.links.is_empty() {
            return Err(BackendError::Internal {
                backend_name: "postgres".to_string(),
                message: "Empty chain".to_string(),
                source: None,
            }
            .into());
        }

        let param_num = self.param_offset + 1;
        let (terminal_sql, terminal_params) =
            self.build_terminal_condition(chain, value, param_num)?;
        let terminal_type = &chain.links[chain.links.len() - 1].target_type;

        // Innermost (terminal) query. `_id` and `_lastUpdated` are not indexed
        // (see `PARAMS_ANSWERED_FROM_RESOURCES`); they are read from the
        // `resources` columns they restate, which is also what the unchained
        // form of the same search does. `is_deleted = FALSE` keeps that read at
        // parity with the index it replaces: a soft delete clears the
        // resource's `search_index` rows, so the subquery never saw one.
        let mut current_sql = if resources_backed(&chain.terminal_param) {
            format!(
                "SELECT '{tt}/' || si{n}.id FROM resources si{n} \
                 WHERE si{n}.tenant_id = $1 AND si{n}.resource_type = '{tt}' \
                 AND si{n}.is_deleted = FALSE AND {cond}",
                tt = terminal_type,
                n = chain.links.len(),
                cond = terminal_sql,
            )
        } else {
            format!(
                "SELECT '{tt}/' || si{n}.resource_id FROM search_index si{n} \
                 WHERE si{n}.tenant_id = $1 AND si{n}.resource_type = '{tt}' \
                 AND si{n}.param_name = '{tp}' AND {cond}",
                tt = terminal_type,
                n = chain.links.len(),
                tp = chain.terminal_param,
                cond = terminal_sql,
            )
        };

        // Wrap with each chain link from innermost to outermost.
        for (i, link) in chain.links.iter().enumerate().rev() {
            let link_num = i + 1;
            let current_type = if i == 0 {
                &self.base_type
            } else {
                &chain.links[i - 1].target_type
            };

            current_sql = if i == 0 {
                // Outermost link: return resource_id for `r.id IN (...)`.
                format!(
                    "SELECT si{ln}.resource_id FROM search_index si{ln} \
                     WHERE si{ln}.tenant_id = $1 AND si{ln}.resource_type = '{ct}' \
                     AND si{ln}.param_name = '{rp}' \
                     AND si{ln}.value_reference IN ({inner})",
                    ln = link_num,
                    ct = current_type,
                    rp = link.reference_param,
                    inner = current_sql,
                )
            } else {
                // Intermediate link: return '{type}/' || resource_id for value_reference matching.
                format!(
                    "SELECT '{ct}/' || si{ln}.resource_id FROM search_index si{ln} \
                     WHERE si{ln}.tenant_id = $1 AND si{ln}.resource_type = '{ct}' \
                     AND si{ln}.param_name = '{rp}' \
                     AND si{ln}.value_reference IN ({inner})",
                    ct = current_type,
                    ln = link_num,
                    rp = link.reference_param,
                    inner = current_sql,
                )
            };
        }

        Ok(SqlFragment::with_params(
            format!("r.id IN ({})", current_sql),
            terminal_params,
        ))
    }

    fn build_terminal_condition(
        &self,
        chain: &ParsedChain,
        value: &SearchValue,
        param_num: usize,
    ) -> StorageResult<(String, Vec<SqlParam>)> {
        let alias = format!("si{}", chain.links.len());

        if let Some((sql, bind)) =
            resources_backed_condition(&chain.terminal_param, &alias, value, param_num)
        {
            return Ok((sql, vec![bind]));
        }

        let (condition, params) = match chain.terminal_type {
            SearchParamType::String => {
                let escaped = value.value.replace('%', "\\%").replace('_', "\\_");
                (
                    format!("{}.value_string ILIKE ${} ESCAPE '\\'", alias, param_num),
                    vec![SqlParam::Text(format!("%{}%", escaped))],
                )
            }
            SearchParamType::Token => {
                if let Some((system, code)) = value.value.split_once('|') {
                    if system.is_empty() {
                        (
                            format!(
                                "({alias}.value_token_system IS NULL OR {alias}.value_token_system = '') \
                                 AND {alias}.value_token_code = ${pn}",
                                alias = alias,
                                pn = param_num,
                            ),
                            vec![SqlParam::Text(code.to_string())],
                        )
                    } else {
                        // Both halves are bound. The system used to be
                        // interpolated into the SQL text with its quotes doubled:
                        // user-supplied text in a literal, correct only for as
                        // long as `standard_conforming_strings` stays on, and a
                        // distinct statement text per system either way.
                        (
                            format!(
                                "{alias}.value_token_system = ${pn} AND {alias}.value_token_code = ${pn2}",
                                alias = alias,
                                pn = param_num,
                                pn2 = param_num + 1,
                            ),
                            vec![
                                SqlParam::Text(system.to_string()),
                                SqlParam::Text(code.to_string()),
                            ],
                        )
                    }
                } else {
                    (
                        format!("{}.value_token_code = ${}", alias, param_num),
                        vec![SqlParam::Text(value.value.clone())],
                    )
                }
            }
            SearchParamType::Reference => (
                format!("{}.value_reference ILIKE ${}", alias, param_num),
                vec![SqlParam::Text(format!("%{}%", value.value))],
            ),
            SearchParamType::Date => {
                let date_col = format!("{}.value_date", alias);
                let (sql, bind) = build_date_condition(&date_col, value, param_num);
                (sql, vec![bind])
            }
            SearchParamType::Number => {
                let num_col = format!("{}.value_number", alias);
                let (sql, bind) = build_number_condition(&num_col, value, param_num);
                (sql, vec![bind])
            }
            SearchParamType::Quantity => {
                let qty_col = format!("{}.value_quantity_value", alias);
                let (sql, bind) = build_number_condition(&qty_col, value, param_num);
                (sql, vec![bind])
            }
            SearchParamType::Uri => (
                format!("{}.value_uri = ${}", alias, param_num),
                vec![SqlParam::Text(value.value.clone())],
            ),
            _ => (
                format!("{}.value_string ILIKE ${}", alias, param_num),
                vec![SqlParam::Text(format!("%{}%", value.value))],
            ),
        };

        Ok((condition, params))
    }

    /// Builds SQL for a reverse chain (`_has`) query.
    ///
    /// For `Patient?_has:Observation:subject:code=1234-5`:
    ///
    /// ```sql
    /// r.id IN (
    ///   SELECT SUBSTRING(si1.value_reference FROM POSITION('/' IN si1.value_reference) + 1)
    ///   FROM search_index si1
    ///   WHERE si1.tenant_id = $1 AND si1.resource_type = 'Observation'
    ///     AND si1.param_name = 'subject'
    ///     AND si1.value_reference LIKE 'Patient/%'
    ///     AND si1.resource_id IN (
    ///       SELECT si2.resource_id FROM search_index si2
    ///       WHERE si2.tenant_id = $1 AND si2.resource_type = 'Observation'
    ///         AND si2.param_name = 'code'
    ///         AND si2.value_token_code = $2
    ///     )
    /// )
    /// ```
    pub fn build_reverse_chain_sql(
        &self,
        reverse_chain: &ReverseChainedParameter,
    ) -> StorageResult<SqlFragment> {
        let depth = reverse_chain.depth();
        if !self.config.validate_reverse_depth(depth) {
            return Err(BackendError::Internal {
                backend_name: "postgres".to_string(),
                message: format!(
                    "Reverse chain depth {} exceeds maximum {}",
                    depth, self.config.max_reverse_depth
                ),
                source: None,
            }
            .into());
        }

        let param_num = self.param_offset + 1;
        let (sql, params) = self.build_reverse_chain_recursive(reverse_chain, 1, param_num)?;

        Ok(SqlFragment::with_params(
            format!("r.id IN ({})", sql),
            params,
        ))
    }

    fn build_reverse_chain_recursive(
        &self,
        rc: &ReverseChainedParameter,
        depth: usize,
        param_num: usize,
    ) -> StorageResult<(String, Vec<SqlParam>)> {
        let alias = format!("si{}", depth);

        if rc.is_terminal() {
            let value = rc.value.as_ref().ok_or_else(|| BackendError::Internal {
                backend_name: "postgres".to_string(),
                message: "Terminal reverse chain must have a value".to_string(),
                source: None,
            })?;

            let (search_condition, search_params) = self.build_reverse_terminal_condition(
                &rc.source_type,
                &rc.search_param,
                value,
                depth + 1,
                param_num,
            )?;

            let depth2 = depth + 1;
            let sql = format!(
                "SELECT SUBSTRING({alias}.value_reference FROM POSITION('/' IN {alias}.value_reference) + 1) \
                 FROM search_index {alias} \
                 WHERE {alias}.tenant_id = $1 AND {alias}.resource_type = '{src_type}' \
                 AND {alias}.param_name = '{ref_param}' \
                 AND {alias}.value_reference LIKE '{base_type}/%' \
                 AND {alias}.resource_id IN ({inner})",
                alias = alias,
                src_type = rc.source_type,
                ref_param = rc.reference_param,
                base_type = self.base_type,
                inner = if resources_backed(&rc.search_param) {
                    // Same substitution as the forward chain's terminal.
                    format!(
                        "SELECT si{depth2}.id FROM resources si{depth2} \
                         WHERE si{depth2}.tenant_id = $1 \
                         AND si{depth2}.resource_type = '{src_type}' \
                         AND si{depth2}.is_deleted = FALSE AND {search_condition}",
                        depth2 = depth2,
                        src_type = rc.source_type,
                        search_condition = search_condition,
                    )
                } else {
                    format!(
                        "SELECT si{depth2}.resource_id FROM search_index si{depth2} \
                         WHERE si{depth2}.tenant_id = $1 \
                         AND si{depth2}.resource_type = '{src_type}' \
                         AND si{depth2}.param_name = '{search_param_name}' \
                         AND {search_condition}",
                        depth2 = depth2,
                        src_type = rc.source_type,
                        search_param_name = rc.search_param,
                        search_condition = search_condition,
                    )
                },
            );

            Ok((sql, search_params))
        } else {
            let inner = rc.nested.as_ref().ok_or_else(|| BackendError::Internal {
                backend_name: "postgres".to_string(),
                message: "Non-terminal reverse chain must have nested chain".to_string(),
                source: None,
            })?;

            let inner_builder = ChainQueryBuilder::new(
                &self.tenant_id,
                &rc.source_type,
                Arc::clone(&self.registry),
            )
            .with_config(self.config.clone())
            .with_param_offset(param_num - 1);

            let (inner_sql, inner_params) =
                inner_builder.build_reverse_chain_recursive(inner, depth + 1, param_num)?;

            let sql = format!(
                "SELECT SUBSTRING({alias}.value_reference FROM POSITION('/' IN {alias}.value_reference) + 1) \
                 FROM search_index {alias} \
                 WHERE {alias}.tenant_id = $1 AND {alias}.resource_type = '{}' \
                 AND {alias}.param_name = '{}' \
                 AND {alias}.value_reference LIKE '{}/%' \
                 AND {alias}.resource_id IN ({inner_sql})",
                rc.source_type,
                rc.reference_param,
                self.base_type,
                alias = alias,
            );

            Ok((sql, inner_params))
        }
    }

    fn build_reverse_terminal_condition(
        &self,
        resource_type: &str,
        param_name: &str,
        value: &SearchValue,
        depth: usize,
        param_num: usize,
    ) -> StorageResult<(String, Vec<SqlParam>)> {
        let param_type = {
            let registry = self.registry.read();
            crate::search::resolve_param_type(
                &registry,
                resource_type,
                param_name,
                std::slice::from_ref(value),
            )
        };

        let alias = format!("si{}", depth);

        if let Some((sql, bind)) = resources_backed_condition(param_name, &alias, value, param_num)
        {
            return Ok((sql, vec![bind]));
        }

        let (condition, params) = match param_type {
            SearchParamType::String => {
                let escaped = value.value.replace('%', "\\%").replace('_', "\\_");
                (
                    format!("{}.value_string ILIKE ${} ESCAPE '\\'", alias, param_num),
                    vec![SqlParam::Text(format!("%{}%", escaped))],
                )
            }
            SearchParamType::Token => {
                if let Some((system, code)) = value.value.split_once('|') {
                    if system.is_empty() {
                        (
                            format!(
                                "({alias}.value_token_system IS NULL OR {alias}.value_token_system = '') \
                                 AND {alias}.value_token_code = ${pn}",
                                alias = alias,
                                pn = param_num,
                            ),
                            vec![SqlParam::Text(code.to_string())],
                        )
                    } else {
                        // Both halves are bound. The system used to be
                        // interpolated into the SQL text with its quotes doubled:
                        // user-supplied text in a literal, correct only for as
                        // long as `standard_conforming_strings` stays on, and a
                        // distinct statement text per system either way.
                        (
                            format!(
                                "{alias}.value_token_system = ${pn} AND {alias}.value_token_code = ${pn2}",
                                alias = alias,
                                pn = param_num,
                                pn2 = param_num + 1,
                            ),
                            vec![
                                SqlParam::Text(system.to_string()),
                                SqlParam::Text(code.to_string()),
                            ],
                        )
                    }
                } else {
                    (
                        format!("{}.value_token_code = ${}", alias, param_num),
                        vec![SqlParam::Text(value.value.clone())],
                    )
                }
            }
            SearchParamType::Reference => (
                format!("{}.value_reference ILIKE ${}", alias, param_num),
                vec![SqlParam::Text(format!("%{}%", value.value))],
            ),
            SearchParamType::Date => {
                let date_col = format!("{}.value_date", alias);
                let (sql, bind) = build_date_condition(&date_col, value, param_num);
                (sql, vec![bind])
            }
            SearchParamType::Number => {
                let num_col = format!("{}.value_number", alias);
                let (sql, bind) = build_number_condition(&num_col, value, param_num);
                (sql, vec![bind])
            }
            SearchParamType::Quantity => {
                let qty_col = format!("{}.value_quantity_value", alias);
                let (sql, bind) = build_number_condition(&qty_col, value, param_num);
                (sql, vec![bind])
            }
            SearchParamType::Uri => (
                format!("{}.value_uri = ${}", alias, param_num),
                vec![SqlParam::Text(value.value.clone())],
            ),
            _ => (
                format!("{}.value_string ILIKE ${}", alias, param_num),
                vec![SqlParam::Text(format!("%{}%", value.value))],
            ),
        };

        Ok((condition, params))
    }
}

fn parse_chain_part(part: &str) -> (String, Option<String>) {
    if let Some((param, type_mod)) = part.split_once(':') {
        (param.to_string(), Some(type_mod.to_string()))
    } else {
        (part.to_string(), None)
    }
}

/// Whether a chain terminal is answered from `resources` rather than
/// `search_index`.
///
/// The list is [`super::writer::PARAMS_ANSWERED_FROM_RESOURCES`], reached
/// through its predicate so the write side and this read side cannot drift:
/// a parameter added there stops being indexed and starts being read from the
/// column in the same edit.
fn resources_backed(param_name: &str) -> bool {
    super::writer::answered_from_resources(param_name)
}

/// The terminal condition for a `resources`-backed chain parameter, against
/// the `resources` columns the parameter restates.
///
/// `Observation?subject:Patient._id=p1` and
/// `Patient?_has:Observation:subject:_lastUpdated=gt2024-01-01` are the shapes
/// this serves. Returns `None` for every other parameter, leaving the caller's
/// `search_index` dispatch untouched.
///
/// `_id` compares `resources.id` for equality, which is what
/// `SearchQueryBuilder::build_id_condition` does for the unchained form, and
/// what the `search_index` row it replaces held in `value_token_code`.
/// `_lastUpdated` reuses the same prefix-aware date comparison the indexed path
/// applies to `value_date`, pointed at `resources.last_updated` — the column the
/// row was a copy of.
fn resources_backed_condition(
    param_name: &str,
    alias: &str,
    value: &SearchValue,
    param_num: usize,
) -> Option<(String, SqlParam)> {
    match param_name {
        "_id" => Some((
            format!("{}.id = ${}", alias, param_num),
            SqlParam::Text(value.value.clone()),
        )),
        "_lastUpdated" => Some(build_date_condition(
            &format!("{}.last_updated", alias),
            value,
            param_num,
        )),
        _ => None,
    }
}

fn build_date_condition(column: &str, value: &SearchValue, param_num: usize) -> (String, SqlParam) {
    use crate::types::SearchPrefix;

    let (op, val) = match value.prefix {
        SearchPrefix::Eq => ("=", &value.value),
        SearchPrefix::Ne => ("!=", &value.value),
        SearchPrefix::Gt => (">", &value.value),
        SearchPrefix::Lt => ("<", &value.value),
        SearchPrefix::Ge => (">=", &value.value),
        SearchPrefix::Le => ("<=", &value.value),
        SearchPrefix::Sa => (">", &value.value),
        SearchPrefix::Eb => ("<", &value.value),
        SearchPrefix::Ap => {
            return (
                format!("DATE({}) = DATE(${})", column, param_num),
                SqlParam::Text(value.value.clone()),
            );
        }
    };

    (
        format!("{} {} ${}", column, op, param_num),
        SqlParam::Text(val.clone()),
    )
}

fn build_number_condition(
    column: &str,
    value: &SearchValue,
    param_num: usize,
) -> (String, SqlParam) {
    use crate::types::SearchPrefix;

    let num_value = value.value.parse::<f64>().unwrap_or(0.0);

    let (op, val) = match value.prefix {
        SearchPrefix::Eq => ("=", num_value),
        SearchPrefix::Ne => ("!=", num_value),
        SearchPrefix::Gt => (">", num_value),
        SearchPrefix::Lt => ("<", num_value),
        SearchPrefix::Ge => (">=", num_value),
        SearchPrefix::Le => ("<=", num_value),
        SearchPrefix::Sa => (">", num_value),
        SearchPrefix::Eb => ("<", num_value),
        SearchPrefix::Ap => {
            let lower = num_value * 0.9;
            let upper = num_value * 1.1;
            return (
                format!("{} BETWEEN {} AND {}", column, lower, upper),
                SqlParam::Float(num_value),
            );
        }
    };

    (
        format!("{} {} ${}", column, op, param_num),
        SqlParam::Float(val),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::SearchParameterDefinition;

    fn registry_with(defs: Vec<SearchParameterDefinition>) -> Arc<RwLock<SearchParameterRegistry>> {
        let mut r = SearchParameterRegistry::new();
        for d in defs {
            r.register(d).unwrap();
        }
        Arc::new(RwLock::new(r))
    }

    fn obs_subject_patient_org_name() -> Arc<RwLock<SearchParameterRegistry>> {
        registry_with(vec![
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Observation-subject",
                "subject",
                SearchParamType::Reference,
                "Observation.subject",
            )
            .with_base(vec!["Observation"])
            .with_targets(vec!["Patient"]),
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Patient-organization",
                "organization",
                SearchParamType::Reference,
                "Patient.managingOrganization",
            )
            .with_base(vec!["Patient"])
            .with_targets(vec!["Organization"]),
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Organization-name",
                "name",
                SearchParamType::String,
                "Organization.name",
            )
            .with_base(vec!["Organization"]),
        ])
    }

    /// `subject.identifier` on Observation (forward) and `code` on Observation
    /// (the reverse-chain terminal) — two token terminals, which is what the
    /// `system|code` binding tests need.
    fn obs_subject_patient_org_code() -> Arc<RwLock<SearchParameterRegistry>> {
        registry_with(vec![
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Observation-subject",
                "subject",
                SearchParamType::Reference,
                "Observation.subject",
            )
            .with_base(vec!["Observation"])
            .with_targets(vec!["Patient"]),
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Patient-identifier",
                "identifier",
                SearchParamType::Token,
                "Patient.identifier",
            )
            .with_base(vec!["Patient"]),
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Observation-code",
                "code",
                SearchParamType::Token,
                "Observation.code",
            )
            .with_base(vec!["Observation"]),
        ])
    }

    #[test]
    fn parses_three_link_chain() {
        let registry = obs_subject_patient_org_name();
        let builder = ChainQueryBuilder::new("t", "Observation", registry);
        let parsed = builder.parse_chain("subject.organization.name").unwrap();
        assert_eq!(parsed.links.len(), 2);
        assert_eq!(parsed.links[0].reference_param, "subject");
        assert_eq!(parsed.links[0].target_type, "Patient");
        assert_eq!(parsed.links[1].reference_param, "organization");
        assert_eq!(parsed.links[1].target_type, "Organization");
        assert_eq!(parsed.terminal_param, "name");
        assert_eq!(parsed.terminal_type, SearchParamType::String);
    }

    #[test]
    fn builds_three_link_chain_sql() {
        let registry = obs_subject_patient_org_name();
        let builder = ChainQueryBuilder::new("t", "Observation", registry);
        let parsed = builder.parse_chain("subject.organization.name").unwrap();
        let value = SearchValue::eq("Hospital");
        let frag = builder.build_forward_chain_sql(&parsed, &value).unwrap();

        assert!(frag.sql.contains("r.id IN ("));
        // Three nested SELECTs (outermost link, intermediate link, terminal).
        // Aliases are si{i+1} per link plus si{links.len()} for the terminal —
        // for a 2-link chain that is si1 (subject), si2 (organization), si2
        // (terminal name); the inner si2 lexically shadows the outer si2.
        assert_eq!(frag.sql.matches("FROM search_index").count(), 3);
        assert!(frag.sql.contains("SELECT si1.resource_id"));
        assert!(frag.sql.contains("'Patient/' || si2.resource_id"));
        assert!(frag.sql.contains("'Organization/' || si2.resource_id"));
        assert!(frag.sql.contains("ILIKE $2 ESCAPE '\\'"));
        assert_eq!(frag.params.len(), 1);
        assert!(matches!(&frag.params[0], SqlParam::Text(s) if s == "%Hospital%"));
    }

    #[test]
    fn chained_id_reads_the_resources_table() {
        // `_id` is no longer indexed (`PARAMS_ANSWERED_FROM_RESOURCES`), so the
        // terminal must resolve against `resources.id`. Reading
        // `search_index.param_name = '_id'` here would silently return nothing.
        let registry = obs_subject_patient_org_name();
        let builder = ChainQueryBuilder::new("t", "Observation", registry);
        let parsed = builder.parse_chain("subject._id").unwrap();
        assert_eq!(parsed.terminal_type, SearchParamType::Token);

        let frag = builder
            .build_forward_chain_sql(&parsed, &SearchValue::eq("p1"))
            .unwrap();

        assert!(
            frag.sql.contains("FROM resources si1"),
            "terminal must read resources: {}",
            frag.sql
        );
        assert!(
            !frag.sql.contains("param_name = '_id'"),
            "no _id rows exist to match: {}",
            frag.sql
        );
        assert!(frag.sql.contains("'Patient/' || si1.id"), "{}", frag.sql);
        assert!(frag.sql.contains("si1.is_deleted = FALSE"), "{}", frag.sql);
        assert!(frag.sql.contains("si1.id = $2"), "{}", frag.sql);
        assert_eq!(frag.params.len(), 1);
        assert!(matches!(&frag.params[0], SqlParam::Text(s) if s == "p1"));
    }

    #[test]
    fn chained_last_updated_reads_the_resources_table() {
        let registry = obs_subject_patient_org_name();
        let builder = ChainQueryBuilder::new("t", "Observation", registry);
        let parsed = builder.parse_chain("subject._lastUpdated").unwrap();
        assert_eq!(parsed.terminal_type, SearchParamType::Date);

        let frag = builder
            .build_forward_chain_sql(&parsed, &SearchValue::parse("gt2024-01-01"))
            .unwrap();

        assert!(frag.sql.contains("FROM resources si1"), "{}", frag.sql);
        assert!(
            frag.sql.contains("si1.last_updated > $2"),
            "the prefix must survive: {}",
            frag.sql
        );
    }

    /// `resources_backed` decides that a chain terminal reads `resources`;
    /// `resources_backed_condition` decides *how*. If a parameter were added to
    /// the writer's list without a column mapping here, the terminal would be
    /// built as `FROM resources` with a predicate naming a `search_index`
    /// column, and the whole chain would error at the database instead of
    /// answering. The two must stay in step.
    #[test]
    fn every_resources_backed_param_has_a_column_mapping() {
        for param in super::super::writer::PARAMS_ANSWERED_FROM_RESOURCES {
            assert!(resources_backed(param), "{param} must be recognised");
            assert!(
                resources_backed_condition(param, "si1", &SearchValue::eq("x"), 2).is_some(),
                "{param} has no `resources` column mapping"
            );
        }
        assert!(resources_backed_condition("code", "si1", &SearchValue::eq("x"), 2).is_none());
    }

    #[test]
    fn an_indexed_chain_terminal_still_reads_search_index() {
        // The substitution must be confined to the two resources-backed
        // parameters; everything else keeps the index path.
        let registry = obs_subject_patient_org_name();
        let builder = ChainQueryBuilder::new("t", "Observation", registry);
        let parsed = builder.parse_chain("subject.organization.name").unwrap();
        let frag = builder
            .build_forward_chain_sql(&parsed, &SearchValue::eq("Hospital"))
            .unwrap();
        assert!(!frag.sql.contains("FROM resources"), "{}", frag.sql);
        assert_eq!(frag.sql.matches("FROM search_index").count(), 3);
    }

    /// The `system|code` token form used to interpolate the SYSTEM into the SQL
    /// text as a literal with its quotes doubled, and bind only the code. Both
    /// halves are request-derived, so that was user text in a string literal:
    /// correct only while `standard_conforming_strings` is on, and a distinct
    /// statement text per system regardless — which is a fresh parse and plan on
    /// every call, and unbounded growth in any prepared-statement cache placed
    /// in front of it.
    ///
    /// A single quote in the system is the thing to test: it must reach the
    /// server as a *value*, never as SQL.
    #[test]
    fn a_chained_token_system_is_bound_not_interpolated() {
        let registry = obs_subject_patient_org_code();
        let builder = ChainQueryBuilder::new("t", "Observation", registry);
        let parsed = builder.parse_chain("subject.identifier").unwrap();
        let frag = builder
            .build_forward_chain_sql(&parsed, &SearchValue::eq("http://ex.org/o'brien|A1"))
            .unwrap();

        assert!(
            frag.sql.contains("value_token_system = $2")
                && frag.sql.contains("value_token_code = $3"),
            "both halves bound: {}",
            frag.sql
        );
        assert!(
            !frag.sql.contains("o'brien") && !frag.sql.contains("o''brien"),
            "the system must not appear in the SQL text at all: {}",
            frag.sql
        );
        assert_eq!(frag.params.len(), 2);
        assert!(matches!(&frag.params[0], SqlParam::Text(v) if v == "http://ex.org/o'brien"));
        assert!(matches!(&frag.params[1], SqlParam::Text(v) if v == "A1"));
    }

    /// Same, on the reverse-chain terminal, which is a separate copy of the
    /// same match.
    #[test]
    fn a_reverse_chained_token_system_is_bound_not_interpolated() {
        let registry = obs_subject_patient_org_code();
        let builder = ChainQueryBuilder::new("t", "Patient", registry);
        let rc = ReverseChainedParameter {
            source_type: "Observation".to_string(),
            reference_param: "subject".to_string(),
            search_param: "code".to_string(),
            value: Some(SearchValue::eq("http://ex.org/o'brien|A1")),
            nested: None,
        };
        let frag = builder.build_reverse_chain_sql(&rc).unwrap();

        assert!(
            frag.sql.contains("value_token_system = $2")
                && frag.sql.contains("value_token_code = $3"),
            "both halves bound: {}",
            frag.sql
        );
        assert!(
            !frag.sql.contains("o'brien") && !frag.sql.contains("o''brien"),
            "the system must not appear in the SQL text at all: {}",
            frag.sql
        );
        assert_eq!(frag.params.len(), 2);
    }

    #[test]
    fn reverse_chain_on_id_reads_the_resources_table() {
        // `Patient?_has:Observation:subject:_id=obs-1`.
        let registry = obs_subject_patient_org_name();
        let builder = ChainQueryBuilder::new("t", "Patient", registry);
        let rc = ReverseChainedParameter {
            source_type: "Observation".to_string(),
            reference_param: "subject".to_string(),
            search_param: "_id".to_string(),
            value: Some(SearchValue::eq("obs-1")),
            nested: None,
        };
        let frag = builder.build_reverse_chain_sql(&rc).unwrap();

        assert!(
            frag.sql.contains("SELECT si2.id FROM resources si2"),
            "{}",
            frag.sql
        );
        assert!(frag.sql.contains("si2.is_deleted = FALSE"), "{}", frag.sql);
        assert!(frag.sql.contains("si2.id = $2"), "{}", frag.sql);
        // The outer link still walks the reference rows in search_index.
        assert!(
            frag.sql.contains("si1.param_name = 'subject'"),
            "{}",
            frag.sql
        );
        assert!(!frag.sql.contains("param_name = '_id'"), "{}", frag.sql);
    }

    #[test]
    fn reverse_chain_on_an_indexed_param_still_reads_search_index() {
        let registry = obs_subject_patient_org_name();
        let builder = ChainQueryBuilder::new("t", "Patient", registry);
        let rc = ReverseChainedParameter {
            source_type: "Observation".to_string(),
            reference_param: "subject".to_string(),
            search_param: "code".to_string(),
            value: Some(SearchValue::eq("1234-5")),
            nested: None,
        };
        let frag = builder.build_reverse_chain_sql(&rc).unwrap();
        assert!(!frag.sql.contains("FROM resources"), "{}", frag.sql);
        assert!(frag.sql.contains("si2.param_name = 'code'"), "{}", frag.sql);
    }

    #[test]
    fn explicit_type_modifier_is_honored() {
        // subject:Patient.name picks Patient even if registry has multiple targets.
        let registry = registry_with(vec![
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Observation-subject",
                "subject",
                SearchParamType::Reference,
                "Observation.subject",
            )
            .with_base(vec!["Observation"])
            .with_targets(vec!["Patient", "Group", "Device", "Location"]),
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Patient-name",
                "name",
                SearchParamType::String,
                "Patient.name",
            )
            .with_base(vec!["Patient"]),
        ]);
        let builder = ChainQueryBuilder::new("t", "Observation", registry);
        let parsed = builder.parse_chain("subject:Patient.name").unwrap();
        assert_eq!(parsed.links[0].target_type, "Patient");
    }

    #[test]
    fn ambiguous_target_falls_back_to_inference() {
        let registry = registry_with(vec![
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Observation-subject",
                "subject",
                SearchParamType::Reference,
                "Observation.subject",
            )
            .with_base(vec!["Observation"])
            .with_targets(vec!["Patient", "Group", "Device", "Location"]),
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Patient-name",
                "name",
                SearchParamType::String,
                "Patient.name",
            )
            .with_base(vec!["Patient"]),
        ]);
        let builder = ChainQueryBuilder::new("t", "Observation", registry);
        let parsed = builder.parse_chain("subject.name").unwrap();
        assert_eq!(parsed.links[0].target_type, "Patient"); // inferred default
    }

    #[test]
    fn empty_chain_errors() {
        let registry = obs_subject_patient_org_name();
        let builder = ChainQueryBuilder::new("t", "Observation", registry);
        assert!(matches!(
            builder.parse_chain(""),
            Err(ChainError::EmptyChain)
        ));
        assert!(matches!(
            builder.parse_chain("just_one_part"),
            Err(ChainError::InvalidSyntax { .. })
        ));
    }

    #[test]
    fn reverse_chain_terminal_sql_uses_substring_position() {
        // Patient?_has:Observation:subject:code=1234-5
        let rc = ReverseChainedParameter {
            source_type: "Observation".to_string(),
            reference_param: "subject".to_string(),
            search_param: "code".to_string(),
            value: Some(SearchValue::eq("1234-5")),
            nested: None,
        };
        let registry = registry_with(vec![
            SearchParameterDefinition::new(
                "http://hl7.org/fhir/SearchParameter/Observation-code",
                "code",
                SearchParamType::Token,
                "Observation.code",
            )
            .with_base(vec!["Observation"]),
        ]);
        let builder = ChainQueryBuilder::new("t", "Patient", registry);
        let frag = builder.build_reverse_chain_sql(&rc).unwrap();
        assert!(frag.sql.contains(
            "SUBSTRING(si1.value_reference FROM POSITION('/' IN si1.value_reference) + 1)"
        ));
        assert!(frag.sql.contains("LIKE 'Patient/%'"));
        assert!(frag.sql.contains("value_token_code = $2"));
    }
}
