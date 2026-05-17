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

        Ok(infer_target_type(ref_param))
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
        let (terminal_sql, terminal_param) =
            self.build_terminal_condition(chain, value, param_num)?;
        let terminal_type = &chain.links[chain.links.len() - 1].target_type;

        // Innermost (terminal) query.
        let mut current_sql = format!(
            "SELECT '{tt}/' || si{n}.resource_id FROM search_index si{n} \
             WHERE si{n}.tenant_id = $1 AND si{n}.resource_type = '{tt}' \
             AND si{n}.param_name = '{tp}' AND {cond}",
            tt = terminal_type,
            n = chain.links.len(),
            tp = chain.terminal_param,
            cond = terminal_sql,
        );

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
            vec![terminal_param],
        ))
    }

    fn build_terminal_condition(
        &self,
        chain: &ParsedChain,
        value: &SearchValue,
        param_num: usize,
    ) -> StorageResult<(String, SqlParam)> {
        let alias = format!("si{}", chain.links.len());

        let (condition, param) = match chain.terminal_type {
            SearchParamType::String => {
                let escaped = value.value.replace('%', "\\%").replace('_', "\\_");
                (
                    format!("{}.value_string ILIKE ${} ESCAPE '\\'", alias, param_num),
                    SqlParam::Text(format!("%{}%", escaped)),
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
                            SqlParam::Text(code.to_string()),
                        )
                    } else {
                        (
                            format!(
                                "{alias}.value_token_system = '{sys}' AND {alias}.value_token_code = ${pn}",
                                alias = alias,
                                sys = system.replace('\'', "''"),
                                pn = param_num,
                            ),
                            SqlParam::Text(code.to_string()),
                        )
                    }
                } else {
                    (
                        format!("{}.value_token_code = ${}", alias, param_num),
                        SqlParam::Text(value.value.clone()),
                    )
                }
            }
            SearchParamType::Reference => (
                format!("{}.value_reference ILIKE ${}", alias, param_num),
                SqlParam::Text(format!("%{}%", value.value)),
            ),
            SearchParamType::Date => {
                let date_col = format!("{}.value_date", alias);
                build_date_condition(&date_col, value, param_num)
            }
            SearchParamType::Number => {
                let num_col = format!("{}.value_number", alias);
                build_number_condition(&num_col, value, param_num)
            }
            SearchParamType::Quantity => {
                let qty_col = format!("{}.value_quantity_value", alias);
                build_number_condition(&qty_col, value, param_num)
            }
            SearchParamType::Uri => (
                format!("{}.value_uri = ${}", alias, param_num),
                SqlParam::Text(value.value.clone()),
            ),
            _ => (
                format!("{}.value_string ILIKE ${}", alias, param_num),
                SqlParam::Text(format!("%{}%", value.value)),
            ),
        };

        Ok((condition, param))
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

            let (search_condition, search_param) = self.build_reverse_terminal_condition(
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
                 AND {alias}.resource_id IN (\
                   SELECT si{depth2}.resource_id FROM search_index si{depth2} \
                   WHERE si{depth2}.tenant_id = $1 AND si{depth2}.resource_type = '{src_type}' \
                   AND si{depth2}.param_name = '{search_param_name}' AND {search_condition}\
                 )",
                alias = alias,
                src_type = rc.source_type,
                ref_param = rc.reference_param,
                base_type = self.base_type,
                depth2 = depth2,
                search_param_name = rc.search_param,
                search_condition = search_condition,
            );

            Ok((sql, vec![search_param]))
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
    ) -> StorageResult<(String, SqlParam)> {
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

        let (condition, param) = match param_type {
            SearchParamType::String => {
                let escaped = value.value.replace('%', "\\%").replace('_', "\\_");
                (
                    format!("{}.value_string ILIKE ${} ESCAPE '\\'", alias, param_num),
                    SqlParam::Text(format!("%{}%", escaped)),
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
                            SqlParam::Text(code.to_string()),
                        )
                    } else {
                        (
                            format!(
                                "{alias}.value_token_system = '{sys}' AND {alias}.value_token_code = ${pn}",
                                alias = alias,
                                sys = system.replace('\'', "''"),
                                pn = param_num,
                            ),
                            SqlParam::Text(code.to_string()),
                        )
                    }
                } else {
                    (
                        format!("{}.value_token_code = ${}", alias, param_num),
                        SqlParam::Text(value.value.clone()),
                    )
                }
            }
            SearchParamType::Reference => (
                format!("{}.value_reference ILIKE ${}", alias, param_num),
                SqlParam::Text(format!("%{}%", value.value)),
            ),
            SearchParamType::Date => {
                let date_col = format!("{}.value_date", alias);
                build_date_condition(&date_col, value, param_num)
            }
            SearchParamType::Number => {
                let num_col = format!("{}.value_number", alias);
                build_number_condition(&num_col, value, param_num)
            }
            SearchParamType::Quantity => {
                let qty_col = format!("{}.value_quantity_value", alias);
                build_number_condition(&qty_col, value, param_num)
            }
            SearchParamType::Uri => (
                format!("{}.value_uri = ${}", alias, param_num),
                SqlParam::Text(value.value.clone()),
            ),
            _ => (
                format!("{}.value_string ILIKE ${}", alias, param_num),
                SqlParam::Text(format!("%{}%", value.value)),
            ),
        };

        Ok((condition, param))
    }
}

fn parse_chain_part(part: &str) -> (String, Option<String>) {
    if let Some((param, type_mod)) = part.split_once(':') {
        (param.to_string(), Some(type_mod.to_string()))
    } else {
        (part.to_string(), None)
    }
}

/// Hardcoded fallback for ambiguous reference targets, matching SQLite's
/// `chain_builder::infer_target_type` so chained queries pick the same
/// default on both backends. See the open-question note in the plan about
/// migrating this to a registry-driven first-target pick later.
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
