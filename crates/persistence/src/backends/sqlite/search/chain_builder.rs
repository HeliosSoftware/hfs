//! Chain Query Builder for FHIR Search.
//!
//! Generates efficient SQL subqueries for:
//! - Forward chained parameters (e.g., `Observation?subject.organization.name=Hospital`)
//! - Reverse chained parameters (_has) (e.g., `Patient?_has:Observation:subject:code=1234-5`)
//!
//! Uses the search_index table to resolve chains efficiently via SQL subqueries
//! instead of in-memory iteration.

use std::sync::Arc;

use parking_lot::RwLock;

use crate::error::{BackendError, StorageResult};
use crate::search::{IMPLICIT_TOKEN_SYSTEM, SearchParameterRegistry};
use crate::types::{ChainConfig, ReverseChainedParameter, SearchParamType, SearchValue};

use super::query_builder::{SqlFragment, SqlParam};

/// A single link in a forward chain.
#[derive(Debug, Clone)]
pub struct ChainLink {
    /// The reference parameter being chained through.
    pub reference_param: String,
    /// The target resource type (resolved from registry or explicit modifier).
    pub target_type: String,
}

/// A parsed forward chain with resolved types.
#[derive(Debug, Clone)]
pub struct ParsedChain {
    /// The chain links from base to target.
    pub links: Vec<ChainLink>,
    /// The terminal parameter name to search on.
    pub terminal_param: String,
    /// The type of the terminal parameter.
    pub terminal_type: SearchParamType,
}

/// Error types specific to chain parsing.
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
    /// Cannot determine target type for reference.
    AmbiguousTargetType {
        /// Resource type the reference parameter belongs to.
        resource_type: String,
        /// Reference parameter name whose target is ambiguous.
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
            } => {
                write!(
                    f,
                    "Unknown reference parameter '{}' for resource type '{}'",
                    param, resource_type
                )
            }
            ChainError::AmbiguousTargetType {
                resource_type,
                param,
            } => {
                write!(
                    f,
                    "Ambiguous target type for parameter '{}' on '{}'. Use type modifier.",
                    param, resource_type
                )
            }
            ChainError::UnknownTerminalParam {
                resource_type,
                param,
            } => {
                write!(
                    f,
                    "Unknown terminal parameter '{}' for resource type '{}'",
                    param, resource_type
                )
            }
            ChainError::EmptyChain => write!(f, "Empty chain"),
            ChainError::InvalidSyntax { message } => write!(f, "Invalid chain syntax: {}", message),
        }
    }
}

impl From<ChainError> for BackendError {
    fn from(e: ChainError) -> Self {
        BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: e.to_string(),
            source: None,
        }
    }
}

/// Builder for chain SQL queries.
///
/// Uses the SearchParameterRegistry to resolve target types for reference
/// parameters and generates efficient SQL subqueries.
pub struct ChainQueryBuilder {
    /// Tenant ID for the query.
    tenant_id: String,
    /// Base resource type being searched.
    base_type: String,
    /// Search parameter registry for type resolution.
    registry: Arc<RwLock<SearchParameterRegistry>>,
    /// Chain depth configuration.
    config: ChainConfig,
    /// Parameter offset for SQL placeholders.
    param_offset: usize,
}

impl ChainQueryBuilder {
    /// Creates a new chain query builder.
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
            param_offset: 2, // Default: after ?1 (tenant) and ?2 (resource_type)
        }
    }

    /// Sets the chain configuration.
    pub fn with_config(mut self, config: ChainConfig) -> Self {
        self.config = config;
        self
    }

    /// Sets the parameter offset for SQL placeholders.
    pub fn with_param_offset(mut self, offset: usize) -> Self {
        self.param_offset = offset;
        self
    }

    /// Parses a chain string into structured chain links.
    ///
    /// # Arguments
    ///
    /// * `chain_str` - The chain path (e.g., "subject.organization.name" or "subject:Patient.name")
    ///
    /// # Returns
    ///
    /// A `ParsedChain` with resolved types, or an error if parsing fails.
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

        // Check depth limit
        let chain_depth = parts.len() - 1; // Last part is terminal param, not a chain link
        if !self.config.validate_forward_depth(chain_depth) {
            return Err(ChainError::MaxDepthExceeded {
                depth: chain_depth,
                max: self.config.max_forward_depth,
            });
        }

        let mut links = Vec::new();
        let mut current_type = self.base_type.clone();

        // Process all parts except the last (which is the terminal parameter)
        for part in parts.iter().take(parts.len() - 1) {
            let (ref_param, explicit_type) = self.parse_chain_part(part);

            // Resolve the target type
            let target_type = self.resolve_target_type(&current_type, &ref_param, explicit_type)?;

            links.push(ChainLink {
                reference_param: ref_param,
                target_type: target_type.clone(),
            });

            current_type = target_type;
        }

        // Get the terminal parameter
        let terminal_param = parts[parts.len() - 1].to_string();
        let terminal_type = self.resolve_terminal_type(&current_type, &terminal_param)?;

        Ok(ParsedChain {
            links,
            terminal_param,
            terminal_type,
        })
    }

    /// Parses a chain part, extracting type modifier if present.
    ///
    /// E.g., "subject:Patient" returns ("subject", Some("Patient"))
    fn parse_chain_part(&self, part: &str) -> (String, Option<String>) {
        if let Some((param, type_mod)) = part.split_once(':') {
            (param.to_string(), Some(type_mod.to_string()))
        } else {
            (part.to_string(), None)
        }
    }

    /// Resolves the target type for a reference parameter.
    ///
    /// Uses the registry to find the parameter definition and its targets.
    /// If the parameter has multiple targets and no explicit type is given,
    /// falls back to inference based on common naming conventions.
    fn resolve_target_type(
        &self,
        resource_type: &str,
        ref_param: &str,
        explicit_type: Option<String>,
    ) -> Result<String, ChainError> {
        // If explicit type is given, use it
        if let Some(t) = explicit_type {
            return Ok(t);
        }

        // Try to resolve from registry
        let registry = self.registry.read();
        if let Some(param_def) = registry.get_param(resource_type, ref_param) {
            // Check if it's a reference parameter
            if param_def.param_type != SearchParamType::Reference {
                return Err(ChainError::UnknownReferenceParam {
                    resource_type: resource_type.to_string(),
                    param: ref_param.to_string(),
                });
            }

            // Get targets
            if let Some(ref targets) = param_def.target {
                if targets.len() == 1 {
                    return Ok(targets[0].clone());
                } else if targets.is_empty() {
                    // Fallback to inference
                    return Ok(crate::search::chain_resolver::infer_target_type(ref_param));
                } else {
                    // Multiple targets - use inference for common patterns
                    // This allows queries like `Observation?subject.name=Smith` to work
                    // by defaulting `subject` to `Patient`
                    return Ok(crate::search::chain_resolver::infer_target_type(ref_param));
                }
            }
        }

        // Fall back to inference based on common parameter names
        Ok(crate::search::chain_resolver::infer_target_type(ref_param))
    }

    /// Resolves the type of the terminal parameter.
    fn resolve_terminal_type(
        &self,
        resource_type: &str,
        param_name: &str,
    ) -> Result<SearchParamType, ChainError> {
        let registry = self.registry.read();
        if let Some(param_def) = registry.get_param(resource_type, param_name) {
            Ok(param_def.param_type)
        } else {
            // Check for common parameters that might not be in registry
            match param_name {
                "_id" | "id" => Ok(SearchParamType::Token),
                "name" | "family" | "given" | "text" | "display" => Ok(SearchParamType::String),
                "identifier" | "code" | "status" | "type" | "category" => {
                    Ok(SearchParamType::Token)
                }
                _ => Err(ChainError::UnknownTerminalParam {
                    resource_type: resource_type.to_string(),
                    param: param_name.to_string(),
                }),
            }
        }
    }

    /// Builds SQL for a forward chain query.
    ///
    /// Generates nested subqueries that efficiently resolve the chain
    /// using the search_index table.
    ///
    /// # Example Output
    ///
    /// For `Observation?subject.organization.name=Hospital`:
    /// ```sql
    /// r.id IN (
    ///   SELECT si1.resource_id FROM search_index si1
    ///   WHERE si1.tenant_id = ?1 AND si1.resource_type = 'Observation'
    ///     AND si1.param_name = 'subject'
    ///     AND si1.value_reference IN (
    ///       SELECT 'Patient/' || si2.resource_id FROM search_index si2
    ///       WHERE si2.tenant_id = ?1 AND si2.resource_type = 'Patient'
    ///         AND si2.param_name = 'organization'
    ///         AND si2.value_reference IN (
    ///           SELECT 'Organization/' || si3.resource_id FROM search_index si3
    ///           WHERE si3.tenant_id = ?1 AND si3.resource_type = 'Organization'
    ///             AND si3.param_name = 'name'
    ///             AND si3.value_string LIKE ?3
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
                backend_name: "sqlite".to_string(),
                message: "Empty chain".to_string(),
                source: None,
            }
            .into());
        }

        // Build from innermost (terminal) to outermost
        let param_num = self.param_offset + 1;

        // Build terminal condition
        let (terminal_sql, terminal_params) =
            self.build_terminal_condition(chain, value, param_num)?;

        // Get the last link to know the terminal resource type
        let terminal_type = &chain.links[chain.links.len() - 1].target_type;

        // Build the innermost query (terminal condition)
        let mut current_sql = format!(
            "SELECT '{}/{}' || si{}.resource_id FROM search_index si{} \
             WHERE si{}.tenant_id = ?1 AND si{}.resource_type = '{}' \
             AND si{}.param_name = '{}' AND {}",
            terminal_type,
            "", // Empty prefix since we concatenate with resource_id
            chain.links.len(),
            chain.links.len(),
            chain.links.len(),
            chain.links.len(),
            terminal_type,
            chain.links.len(),
            chain.terminal_param,
            terminal_sql
        );

        // Wrap with each chain link from innermost to outermost
        for (i, link) in chain.links.iter().enumerate().rev() {
            let link_num = i + 1;
            // current_type is the resource type that contains this reference param
            let current_type = if i == 0 {
                &self.base_type
            } else {
                &chain.links[i - 1].target_type
            };

            if i == 0 {
                // Outermost link: return just resource_id for r.id IN (...)
                current_sql = format!(
                    "SELECT si{link_num}.resource_id FROM search_index si{link_num} \
                     WHERE si{link_num}.tenant_id = ?1 AND si{link_num}.resource_type = '{current_type}' \
                     AND si{link_num}.param_name = '{ref_param}' \
                     AND si{link_num}.value_reference IN ({inner})",
                    link_num = link_num,
                    current_type = current_type,
                    ref_param = link.reference_param,
                    inner = current_sql
                );
            } else {
                // Intermediate link: return '{type}/' || resource_id for value_reference matching
                current_sql = format!(
                    "SELECT '{current_type}/' || si{link_num}.resource_id FROM search_index si{link_num} \
                     WHERE si{link_num}.tenant_id = ?1 AND si{link_num}.resource_type = '{current_type}' \
                     AND si{link_num}.param_name = '{ref_param}' \
                     AND si{link_num}.value_reference IN ({inner})",
                    current_type = current_type,
                    link_num = link_num,
                    ref_param = link.reference_param,
                    inner = current_sql
                );
            }
        }

        // Final wrap to select matching base resource IDs
        let final_sql = format!("r.id IN ({})", current_sql);

        Ok(SqlFragment::with_params(final_sql, terminal_params))
    }

    /// Builds the terminal condition for a chain query.
    fn build_terminal_condition(
        &self,
        chain: &ParsedChain,
        value: &SearchValue,
        param_num: usize,
    ) -> StorageResult<(String, Vec<SqlParam>)> {
        let alias_num = chain.links.len();
        let alias = format!("si{}", alias_num);

        // Number and quantity bind zero, one or several parameters; every other
        // type binds exactly one.
        let (condition, param) = match chain.terminal_type {
            SearchParamType::String => {
                let escaped = value.value.replace('%', "\\%").replace('_', "\\_");
                (
                    format!("{}.value_string LIKE ?{} ESCAPE '\\'", alias, param_num),
                    SqlParam::String(format!("%{}%", escaped)),
                )
            }
            SearchParamType::Token => {
                // Handle system|code format
                if let Some((system, code)) = value.value.split_once('|') {
                    if system.is_empty() {
                        (
                            format!(
                                "({}.value_token_system IS NULL OR {}.value_token_system IN ('', '{}')) \
                                 AND {}.value_token_code = ?{}",
                                alias, alias, IMPLICIT_TOKEN_SYSTEM, alias, param_num
                            ),
                            SqlParam::String(code.to_string()),
                        )
                    } else {
                        (
                            format!(
                                "{}.value_token_system IN ('{}', '{}') AND {}.value_token_code = ?{}",
                                alias,
                                system.replace('\'', "''"),
                                IMPLICIT_TOKEN_SYSTEM,
                                alias,
                                param_num
                            ),
                            SqlParam::String(code.to_string()),
                        )
                    }
                } else {
                    (
                        format!("{}.value_token_code = ?{}", alias, param_num),
                        SqlParam::String(value.value.clone()),
                    )
                }
            }
            SearchParamType::Reference => (
                format!("{}.value_reference LIKE ?{}", alias, param_num),
                SqlParam::String(format!("%{}%", value.value)),
            ),
            SearchParamType::Date => {
                // For date, use range comparison based on prefix
                let date_col = format!("{}.value_date", alias);
                build_date_condition(&date_col, value, param_num)
            }
            SearchParamType::Number => {
                return Ok(build_number_condition(&alias, value, param_num));
            }
            SearchParamType::Quantity => {
                return Ok(build_quantity_condition(&alias, value, param_num));
            }
            SearchParamType::Uri => (
                format!("{}.value_uri = ?{}", alias, param_num),
                SqlParam::String(value.value.clone()),
            ),
            _ => (
                format!("{}.value_string LIKE ?{}", alias, param_num),
                SqlParam::String(format!("%{}%", value.value)),
            ),
        };

        Ok((condition, vec![param]))
    }

    /// Builds SQL for a reverse chain (_has) query.
    ///
    /// Generates subqueries that find base resources referenced by
    /// resources matching the search criteria.
    ///
    /// # Example Output
    ///
    /// For `Patient?_has:Observation:subject:code=1234-5`:
    /// ```sql
    /// r.id IN (
    ///   SELECT SUBSTR(si1.value_reference, INSTR(si1.value_reference, '/') + 1)
    ///   FROM search_index si1
    ///   WHERE si1.tenant_id = ?1 AND si1.resource_type = 'Observation'
    ///     AND si1.param_name = 'subject'
    ///     AND si1.value_reference LIKE 'Patient/%'
    ///     AND si1.resource_id IN (
    ///       SELECT si2.resource_id FROM search_index si2
    ///       WHERE si2.tenant_id = ?1 AND si2.resource_type = 'Observation'
    ///         AND si2.param_name = 'code'
    ///         AND si2.value_token_code = ?3
    ///     )
    /// )
    /// ```
    pub fn build_reverse_chain_sql(
        &self,
        reverse_chain: &ReverseChainedParameter,
    ) -> StorageResult<SqlFragment> {
        // Check depth limit
        let depth = reverse_chain.depth();
        if !self.config.validate_reverse_depth(depth) {
            return Err(BackendError::Internal {
                backend_name: "sqlite".to_string(),
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

    /// Recursively builds reverse chain SQL.
    fn build_reverse_chain_recursive(
        &self,
        rc: &ReverseChainedParameter,
        depth: usize,
        param_num: usize,
    ) -> StorageResult<(String, Vec<SqlParam>)> {
        let alias = format!("si{}", depth);

        if rc.is_terminal() {
            // Terminal case: has a search parameter and value
            let value = rc.value.as_ref().ok_or_else(|| BackendError::Internal {
                backend_name: "sqlite".to_string(),
                message: "Terminal reverse chain must have a value".to_string(),
                source: None,
            })?;

            // Build the search condition for the terminal parameter
            let (search_condition, search_params) = self.build_reverse_terminal_condition(
                &rc.source_type,
                &rc.search_param,
                value,
                depth + 1,
                param_num,
            )?;

            // Build the reference extraction query
            let depth2 = depth + 1;
            let sql = format!(
                "SELECT SUBSTR({alias}.value_reference, INSTR({alias}.value_reference, '/') + 1) \
                 FROM search_index {alias} \
                 WHERE {alias}.tenant_id = ?1 AND {alias}.resource_type = '{src_type}' \
                 AND {alias}.param_name = '{ref_param}' \
                 AND {alias}.value_reference LIKE '{base_type}/%' \
                 AND {alias}.resource_id IN (\
                   SELECT si{depth2}.resource_id FROM search_index si{depth2} \
                   WHERE si{depth2}.tenant_id = ?1 AND si{depth2}.resource_type = '{src_type}' \
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

            Ok((sql, search_params))
        } else {
            // Nested case: recurse into inner _has
            let inner = rc.nested.as_ref().ok_or_else(|| BackendError::Internal {
                backend_name: "sqlite".to_string(),
                message: "Non-terminal reverse chain must have nested chain".to_string(),
                source: None,
            })?;

            // The inner chain's base type is this chain's source type
            let inner_builder = ChainQueryBuilder::new(
                &self.tenant_id,
                &rc.source_type,
                Arc::clone(&self.registry),
            )
            .with_config(self.config.clone())
            .with_param_offset(param_num - 1);

            let (inner_sql, inner_params) =
                inner_builder.build_reverse_chain_recursive(inner, depth + 1, param_num)?;

            // Build the reference extraction query that wraps the inner query
            let sql = format!(
                "SELECT SUBSTR({alias}.value_reference, INSTR({alias}.value_reference, '/') + 1) \
                 FROM search_index {alias} \
                 WHERE {alias}.tenant_id = ?1 AND {alias}.resource_type = '{}' \
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

    /// Builds the terminal condition for a reverse chain search parameter.
    fn build_reverse_terminal_condition(
        &self,
        resource_type: &str,
        param_name: &str,
        value: &SearchValue,
        depth: usize,
        param_num: usize,
    ) -> StorageResult<(String, Vec<SqlParam>)> {
        // Determine the parameter type from the registry. Falls back to the
        // shared value-shape heuristic for unregistered custom params.
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
                    format!("{}.value_string LIKE ?{} ESCAPE '\\'", alias, param_num),
                    SqlParam::String(format!("%{}%", escaped)),
                )
            }
            SearchParamType::Token => {
                if let Some((system, code)) = value.value.split_once('|') {
                    if system.is_empty() {
                        (
                            format!(
                                "({}.value_token_system IS NULL OR {}.value_token_system IN ('', '{}')) \
                                 AND {}.value_token_code = ?{}",
                                alias, alias, IMPLICIT_TOKEN_SYSTEM, alias, param_num
                            ),
                            SqlParam::String(code.to_string()),
                        )
                    } else {
                        (
                            format!(
                                "{}.value_token_system IN ('{}', '{}') AND {}.value_token_code = ?{}",
                                alias,
                                system.replace('\'', "''"),
                                IMPLICIT_TOKEN_SYSTEM,
                                alias,
                                param_num
                            ),
                            SqlParam::String(code.to_string()),
                        )
                    }
                } else {
                    (
                        format!("{}.value_token_code = ?{}", alias, param_num),
                        SqlParam::String(value.value.clone()),
                    )
                }
            }
            SearchParamType::Reference => (
                format!("{}.value_reference LIKE ?{}", alias, param_num),
                SqlParam::String(format!("%{}%", value.value)),
            ),
            SearchParamType::Date => {
                let date_col = format!("{}.value_date", alias);
                build_date_condition(&date_col, value, param_num)
            }
            SearchParamType::Number => {
                return Ok(build_number_condition(&alias, value, param_num));
            }
            SearchParamType::Quantity => {
                return Ok(build_quantity_condition(&alias, value, param_num));
            }
            SearchParamType::Uri => (
                format!("{}.value_uri = ?{}", alias, param_num),
                SqlParam::String(value.value.clone()),
            ),
            _ => (
                format!("{}.value_string LIKE ?{}", alias, param_num),
                SqlParam::String(format!("%{}%", value.value)),
            ),
        };

        Ok((condition, vec![param]))
    }
}

/// Builds a date comparison condition.
fn build_date_condition(column: &str, value: &SearchValue, param_num: usize) -> (String, SqlParam) {
    // Matches nothing, still binding `?param_num`, for a value that is not a
    // date — which the search gate rejects before a chain is ever built.
    let (sql, bound) = super::parameter_handlers::date::date_condition_or_nothing(
        column,
        value.prefix,
        &value.value,
        param_num,
    );
    (sql, SqlParam::String(bound))
}

/// The terminal `number` comparison, against `{alias}.value_number`.
///
/// Delegates to [`NumberHandler`](super::parameter_handlers::NumberHandler),
/// the unchained `number` search's handler, so a chained number means what the
/// unchained one does: the implicit-precision range for `eq`/`ne` (`100` is
/// `[99.5, 100.5)`), the exact value for the comparators, for `ap` a bound
/// `BETWEEN` whose margin is taken from the magnitude (so a negative value has
/// its bounds in order), and `1 = 0` for a value that is not a number.
///
/// This used to be its own operator table. Its `ap` arm wrote both bounds into
/// the SQL text and still returned a bind, so rusqlite refused the statement
/// with `Wrong number of parameters passed to query. Got 3, needed 2`; for a
/// negative value the inlined bounds were also reversed. It read an unparseable
/// number as `0` (#1306).
///
/// Binds zero, one or two parameters, numbered `?N` from `param_num` with no
/// gaps. The terminal condition is the only part of a chain that binds
/// anything (links use `?1` and literals), so the caller only has to return
/// these params in order.
fn build_number_condition(
    alias: &str,
    value: &SearchValue,
    param_num: usize,
) -> (String, Vec<SqlParam>) {
    let fragment = super::parameter_handlers::NumberHandler::build_sql_for(
        &format!("{alias}.value_number"),
        value,
        param_num - 1,
    );
    (format!("({})", fragment.sql), fragment.params)
}

/// The terminal `quantity` comparison, against `{alias}.value_quantity_*`.
///
/// Delegates to [`QuantityHandler`](super::parameter_handlers::QuantityHandler),
/// the unchained `quantity` search's handler: `number`, `number|code` and
/// `number|system|code` are all read, the unit and system are compared as
/// stored, and a convertible UCUM unit also matches its equivalents through the
/// canonical columns.
///
/// This used to share [`build_number_condition`]'s operator table and its
/// defects (#1306), and parsed the whole value as the number, so any value
/// carrying a unit was read as `0`.
fn build_quantity_condition(
    alias: &str,
    value: &SearchValue,
    param_num: usize,
) -> (String, Vec<SqlParam>) {
    let fragment = super::parameter_handlers::QuantityHandler::build_sql_for(
        &format!("{alias}."),
        value,
        param_num - 1,
    );
    (format!("({})", fragment.sql), fragment.params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::SearchParameterDefinition;

    fn create_test_registry() -> Arc<RwLock<SearchParameterRegistry>> {
        let mut registry = SearchParameterRegistry::new();

        // Add some test parameters
        let patient_subject = SearchParameterDefinition::new(
            "http://hl7.org/fhir/SearchParameter/Observation-subject",
            "subject",
            SearchParamType::Reference,
            "Observation.subject",
        )
        .with_base(vec!["Observation"])
        .with_targets(vec!["Patient"]);

        let patient_org = SearchParameterDefinition::new(
            "http://hl7.org/fhir/SearchParameter/Patient-organization",
            "organization",
            SearchParamType::Reference,
            "Patient.managingOrganization",
        )
        .with_base(vec!["Patient"])
        .with_targets(vec!["Organization"]);

        let org_name = SearchParameterDefinition::new(
            "http://hl7.org/fhir/SearchParameter/Organization-name",
            "name",
            SearchParamType::String,
            "Organization.name",
        )
        .with_base(vec!["Organization"]);

        let patient_name = SearchParameterDefinition::new(
            "http://hl7.org/fhir/SearchParameter/Patient-name",
            "name",
            SearchParamType::String,
            "Patient.name",
        )
        .with_base(vec!["Patient"]);

        let obs_code = SearchParameterDefinition::new(
            "http://hl7.org/fhir/SearchParameter/Observation-code",
            "code",
            SearchParamType::Token,
            "Observation.code",
        )
        .with_base(vec!["Observation"]);

        registry.register(patient_subject).unwrap();
        registry.register(patient_org).unwrap();
        registry.register(org_name).unwrap();
        registry.register(patient_name).unwrap();
        registry.register(obs_code).unwrap();

        Arc::new(RwLock::new(registry))
    }

    #[test]
    fn test_parse_simple_chain() {
        let registry = create_test_registry();
        let builder = ChainQueryBuilder::new("tenant1", "Observation", registry);

        let result = builder.parse_chain("subject.name");
        assert!(result.is_ok());

        let chain = result.unwrap();
        assert_eq!(chain.links.len(), 1);
        assert_eq!(chain.links[0].reference_param, "subject");
        assert_eq!(chain.links[0].target_type, "Patient");
        assert_eq!(chain.terminal_param, "name");
        assert_eq!(chain.terminal_type, SearchParamType::String);
    }

    #[test]
    fn test_parse_multi_level_chain() {
        let registry = create_test_registry();
        let builder = ChainQueryBuilder::new("tenant1", "Observation", registry);

        let result = builder.parse_chain("subject.organization.name");
        assert!(result.is_ok());

        let chain = result.unwrap();
        assert_eq!(chain.links.len(), 2);
        assert_eq!(chain.links[0].reference_param, "subject");
        assert_eq!(chain.links[0].target_type, "Patient");
        assert_eq!(chain.links[1].reference_param, "organization");
        assert_eq!(chain.links[1].target_type, "Organization");
        assert_eq!(chain.terminal_param, "name");
    }

    #[test]
    fn test_parse_chain_with_type_modifier() {
        let registry = create_test_registry();
        let builder = ChainQueryBuilder::new("tenant1", "Observation", registry);

        let result = builder.parse_chain("subject:Patient.name");
        assert!(result.is_ok());

        let chain = result.unwrap();
        assert_eq!(chain.links[0].target_type, "Patient");
    }

    #[test]
    fn test_max_depth_exceeded() {
        let registry = create_test_registry();
        let builder = ChainQueryBuilder::new("tenant1", "Observation", registry)
            .with_config(ChainConfig::new(2, 2));

        let result = builder.parse_chain("a.b.c.d"); // 3 chain links
        assert!(matches!(
            result,
            Err(ChainError::MaxDepthExceeded { depth: 3, max: 2 })
        ));
    }

    #[test]
    fn test_build_forward_chain_sql() {
        let registry = create_test_registry();
        let builder = ChainQueryBuilder::new("tenant1", "Observation", registry);

        let chain = builder.parse_chain("subject.name").unwrap();
        let value = SearchValue::eq("Smith");

        let result = builder.build_forward_chain_sql(&chain, &value);
        assert!(result.is_ok());

        let fragment = result.unwrap();
        assert!(fragment.sql.contains("r.id IN"));
        assert!(fragment.sql.contains("search_index"));
        assert!(fragment.sql.contains("subject"));
        assert!(fragment.sql.contains("name"));
    }

    #[test]
    fn test_build_reverse_chain_sql() {
        let registry = create_test_registry();
        let builder = ChainQueryBuilder::new("tenant1", "Patient", registry);

        let rc = ReverseChainedParameter::terminal(
            "Observation",
            "subject",
            "code",
            SearchValue::eq("1234-5"),
        );

        let result = builder.build_reverse_chain_sql(&rc);
        assert!(result.is_ok());

        let fragment = result.unwrap();
        assert!(fragment.sql.contains("r.id IN"));
        assert!(fragment.sql.contains("Observation"));
        assert!(fragment.sql.contains("subject"));
        assert!(fragment.sql.contains("code"));
        assert!(fragment.sql.contains("Patient/%"));
    }

    /// #1379: the chain terminal is a separate copy of the token match, and
    /// has to accept the implicit-system marker of a `code` element the same
    /// way `TokenHandler` does — for `system|code` and for `|code`.
    #[test]
    fn a_chained_system_qualified_token_accepts_the_implicit_system() {
        let registry = create_test_registry();
        let builder = ChainQueryBuilder::new("tenant1", "Patient", registry);
        for (value, expected) in [
            (
                "http://loinc.org|1234-5",
                format!("value_token_system IN ('http://loinc.org', '{IMPLICIT_TOKEN_SYSTEM}')"),
            ),
            (
                "|1234-5",
                format!("value_token_system IN ('', '{IMPLICIT_TOKEN_SYSTEM}')"),
            ),
        ] {
            let rc = ReverseChainedParameter::terminal(
                "Observation",
                "subject",
                "code",
                SearchValue::eq(value),
            );
            let fragment = builder.build_reverse_chain_sql(&rc).unwrap();
            assert!(fragment.sql.contains(&expected), "{}", fragment.sql);
        }
    }

    #[test]
    fn test_reverse_chain_depth() {
        let inner = ReverseChainedParameter::terminal(
            "Provenance",
            "target",
            "agent",
            SearchValue::eq("Practitioner/123"),
        );
        let outer = ReverseChainedParameter::nested("Observation", "subject", inner);

        assert_eq!(outer.depth(), 2);
        assert!(!outer.is_terminal());
    }
}

#[cfg(test)]
mod date_condition_tests {
    use super::*;
    use crate::types::SearchPrefix;

    /// #456: chained date terminals use the precision-aware normalized
    /// comparison, not the raw text `=` this used to emit.
    #[test]
    fn chained_dates_are_precision_aware() {
        let value = SearchValue::new(SearchPrefix::Eq, "1995-10-02");
        let (sql, param) = build_date_condition("t2.value_date", &value, 7);
        assert_eq!(
            sql,
            "(datetime(t2.value_date) >= datetime(?7) AND datetime(t2.value_date) < datetime(?7, '+1 day'))"
        );
        match param {
            SqlParam::String(s) => assert_eq!(s, "1995-10-02T00:00:00"),
            _ => panic!("expected string param"),
        }
    }

    #[test]
    fn chained_full_precision_is_equality() {
        let value = SearchValue::new(SearchPrefix::Eq, "2016-01-23T13:07:42-04:00");
        let (sql, _) = build_date_condition("t2.value_date", &value, 3);
        assert_eq!(sql, "datetime(t2.value_date) = datetime(?3)");
    }
}

#[cfg(test)]
mod numeric_condition_tests {
    use super::*;

    /// The `?N` numbers a fragment references, sorted and deduplicated.
    fn placeholders(sql: &str) -> Vec<usize> {
        let mut found = Vec::new();
        let mut rest = sql;
        while let Some(at) = rest.find('?') {
            rest = &rest[at + 1..];
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            found.push(digits.parse().expect("every placeholder is numbered"));
        }
        found.sort_unstable();
        found.dedup();
        found
    }

    fn floats(params: &[SqlParam]) -> Vec<f64> {
        params
            .iter()
            .filter_map(|p| match p {
                SqlParam::Float(f) => Some(*f),
                _ => None,
            })
            .collect()
    }

    fn assert_floats(got: &[f64], want: &[f64], context: &str) {
        assert_eq!(got.len(), want.len(), "{context}: {got:?}");
        for (g, w) in got.iter().zip(want) {
            assert!((g - w).abs() < 1e-9, "{context}: {got:?} != {want:?}");
        }
    }

    /// #1306: every prefix of a chained number binds what it references, with
    /// placeholders numbered from `param_num` without gaps. `ap` used to inline
    /// its bounds and still return a bind.
    #[test]
    fn a_chained_number_binds_every_bound_with_gap_free_placeholders() {
        let cases: &[(&str, &str, &[f64])] = &[
            (
                "100",
                "(si1.value_number >= ?3 AND si1.value_number < ?4)",
                &[99.5, 100.5],
            ),
            (
                "ne100",
                "((si1.value_number < ?3 OR si1.value_number >= ?4))",
                &[99.5, 100.5],
            ),
            ("gt100", "(si1.value_number > ?3)", &[100.0]),
            ("ge100", "(si1.value_number >= ?3)", &[100.0]),
            ("lt100", "(si1.value_number < ?3)", &[100.0]),
            ("le100", "(si1.value_number <= ?3)", &[100.0]),
            ("sa100", "(si1.value_number > ?3)", &[100.0]),
            ("eb100", "(si1.value_number < ?3)", &[100.0]),
            (
                "ap100",
                "(si1.value_number BETWEEN ?3 AND ?4)",
                &[90.0, 110.0],
            ),
            // Negative: the lower bound is still the smaller one.
            (
                "ap-100",
                "(si1.value_number BETWEEN ?3 AND ?4)",
                &[-110.0, -90.0],
            ),
        ];
        for (value, predicate, binds) in cases {
            let (sql, params) = build_number_condition("si1", &SearchValue::parse(value), 3);
            assert_eq!(&sql, predicate, "{value}");
            assert_floats(&floats(&params), binds, value);
            assert_eq!(params.len(), binds.len(), "{value}: {params:?}");
            let expected: Vec<usize> = (3..3 + binds.len()).collect();
            assert_eq!(placeholders(&sql), expected, "{value}: {sql}");
        }
    }

    /// #1306: a chained quantity reads `number|system|code` as the unchained
    /// search does, against aliased columns, still without placeholder gaps.
    #[test]
    fn a_chained_quantity_qualifies_every_column_and_binds_every_placeholder() {
        let (sql, params) = build_quantity_condition("si2", &SearchValue::parse("ap-5.4"), 3);
        assert_eq!(sql, "(si2.value_quantity_value BETWEEN ?3 AND ?4)");
        assert_floats(&floats(&params), &[-5.94, -4.86], "ap-5.4");

        for value in [
            "5.4|http://unitsofmeasure.org|mg",
            "ap5.4||mg",
            "ne5.4|mg",
            "gt5.4|http://unitsofmeasure.org|mg",
            "5.4||not-a-ucum-unit",
        ] {
            let (sql, params) = build_quantity_condition("si2", &SearchValue::parse(value), 3);
            let expected: Vec<usize> = (3..3 + params.len()).collect();
            assert_eq!(placeholders(&sql), expected, "{value}: {sql}");
            assert_eq!(
                sql.matches("value_quantity_").count(),
                sql.matches("si2.value_quantity_").count(),
                "{value}: {sql}"
            );
        }
    }

    /// #1306: a value that is not a number matches nothing and binds nothing,
    /// under `ne` too. It used to be compared as `0`.
    #[test]
    fn a_chained_non_number_matches_nothing_and_binds_nothing() {
        for value in ["abc", "apabc", "neabc", "gt", "", "abc||mg", "|mg"] {
            for build in [build_number_condition, build_quantity_condition] {
                let (sql, params) = build("si1", &SearchValue::parse(value), 3);
                assert_eq!(sql, "(1 = 0)", "{value}");
                assert!(params.is_empty(), "{value}: {params:?}");
            }
        }
    }
}

/// Chained number and quantity terminals through the `ChainedSearchProvider`
/// trait API (#1306), the only way this builder is reached. They live here
/// rather than beside the other trait-API tests in `search_impl.rs` because the
/// defect and its fix are this file's.
#[cfg(test)]
mod numeric_terminal_tests {
    use crate::backends::sqlite::SqliteBackend;
    use crate::core::{ChainedSearchProvider, ResourceStorage, SearchProvider};
    use crate::tenant::{TenantContext, TenantId, TenantPermissions};
    use crate::types::{
        ReverseChainedParameter, SearchParamType, SearchParameter, SearchQuery, SearchValue,
    };
    use helios_fhir::FhirVersion;
    use serde_json::json;

    fn tenant() -> TenantContext {
        TenantContext::new(
            TenantId::new("chain-numeric"),
            TenantPermissions::full_access(),
        )
    }

    /// Seeds the fixture through the real write path, on a backend that loads
    /// the full R4 search-parameter set (without the data dir only five
    /// embedded params exist and nothing numeric is indexed).
    ///
    /// Number terminal, `MolecularSequence.referenceSeq.windowStart`, reached
    /// by `Observation?has-member:MolecularSequence.window-start`:
    /// `on-neg` → -100, `on-zero` → 0, `on-100` → 100, `on-105` → 105,
    /// `on-200` → 200. The zero row is what an unparseable number used to
    /// match, because it was read as `0`.
    ///
    /// Quantity terminal, `Observation.valueQuantity`, reached by
    /// `DiagnosticReport?result:Observation.value-quantity` and by
    /// `Patient?_has:Observation:subject:value-quantity`:
    /// `dr-neg`/`pq-neg` → -5.4 mg, `dr-a`/`pq-a` → 5.4 mg,
    /// `dr-b`/`pq-b` → 5.9 mg, `dr-c`/`pq-c` → 6.5 mg,
    /// `dr-g`/`pq-g` → 0.0054 g (the same amount as 5.4 mg).
    ///
    /// Number terminal for the reverse direction,
    /// `Patient?_has:ChargeItem:subject:factor-override`: `pq-a` → 2,
    /// `pq-neg` → -2, `pq-c` → 0.
    ///
    /// Asserts that the *unchained* number and quantity searches find the
    /// seeded rows, so no chained assertion can pass vacuously.
    async fn seeded_backend() -> SqliteBackend {
        let data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("data");
        let mut config = crate::backends::sqlite::backend::SqliteBackendConfig::default();
        config.data_dir = Some(data_dir);
        let backend = SqliteBackend::with_config(":memory:", config).unwrap();
        backend.init_schema().unwrap();
        let tenant = tenant();

        let mut resources = Vec::new();
        for (suffix, window_start) in [
            ("neg", -100),
            ("zero", 0),
            ("100", 100),
            ("105", 105),
            ("200", 200),
        ] {
            resources.push((
                "MolecularSequence",
                json!({
                    "resourceType": "MolecularSequence",
                    "id": format!("ms-{suffix}"),
                    "coordinateSystem": 0,
                    "referenceSeq": {"windowStart": window_start, "windowEnd": 1000},
                }),
            ));
            resources.push((
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": format!("on-{suffix}"),
                    "status": "final",
                    "code": {"text": "x"},
                    "hasMember": [{"reference": format!("MolecularSequence/ms-{suffix}")}],
                }),
            ));
        }
        for (suffix, amount, unit) in [
            ("neg", -5.4, "mg"),
            ("a", 5.4, "mg"),
            ("b", 5.9, "mg"),
            ("c", 6.5, "mg"),
            ("g", 0.0054, "g"),
        ] {
            resources.push((
                "Patient",
                json!({"resourceType": "Patient", "id": format!("pq-{suffix}")}),
            ));
            resources.push((
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": format!("oq-{suffix}"),
                    "status": "final",
                    "code": {"text": "x"},
                    "subject": {"reference": format!("Patient/pq-{suffix}")},
                    "valueQuantity": {
                        "value": amount,
                        "unit": unit,
                        "system": "http://unitsofmeasure.org",
                        "code": unit,
                    },
                }),
            ));
            resources.push((
                "DiagnosticReport",
                json!({
                    "resourceType": "DiagnosticReport",
                    "id": format!("dr-{suffix}"),
                    "status": "final",
                    "code": {"text": "x"},
                    "result": [{"reference": format!("Observation/oq-{suffix}")}],
                }),
            ));
        }
        for (patient, factor) in [("pq-a", 2.0), ("pq-neg", -2.0), ("pq-c", 0.0)] {
            resources.push((
                "ChargeItem",
                json!({
                    "resourceType": "ChargeItem",
                    "id": format!("ci-{patient}"),
                    "status": "billable",
                    "code": {"text": "x"},
                    "subject": {"reference": format!("Patient/{patient}")},
                    "factorOverride": factor,
                }),
            ));
        }
        for (resource_type, resource) in resources {
            backend
                .create(&tenant, resource_type, resource, FhirVersion::default())
                .await
                .unwrap();
        }

        for (resource_type, name, param_type, value) in [
            (
                "MolecularSequence",
                "window-start",
                SearchParamType::Number,
                "ap100",
            ),
            (
                "Observation",
                "value-quantity",
                SearchParamType::Quantity,
                "ap5.4",
            ),
        ] {
            let query = SearchQuery::new(resource_type).with_parameter(SearchParameter {
                name: name.to_string(),
                param_type,
                modifier: None,
                values: vec![SearchValue::parse(value)],
                chain: vec![],
                components: vec![],
            });
            let found = backend.search(&tenant, &query).await.unwrap();
            assert_eq!(
                found.resources.items.len(),
                2,
                "fixture: {name} must be indexed or the chained tests are vacuous"
            );
        }
        backend
    }

    /// Compares one case, recording rather than panicking so a run reports
    /// every failing prefix at once.
    fn check(
        failures: &mut Vec<String>,
        label: &str,
        value: &str,
        result: Result<Vec<String>, impl std::fmt::Display>,
        expected: &[&str],
    ) {
        match result {
            Err(e) => failures.push(format!("{label}={value}: {e}")),
            Ok(mut ids) => {
                ids.sort();
                ids.dedup();
                if ids != expected {
                    failures.push(format!("{label}={value}: got {ids:?}, want {expected:?}"));
                }
            }
        }
    }

    #[tokio::test]
    async fn resolve_chain_number_terminal() {
        let backend = seeded_backend().await;
        let cases: &[(&str, &[&str])] = &[
            ("ap100", &["on-100", "on-105"]),
            // A negative value: the window is [-110, -90], not [-90, -110].
            ("ap-100", &["on-neg"]),
            ("ap0", &["on-zero"]),
            ("100", &["on-100"]),
            ("100.0", &["on-100"]),
            ("ne100", &["on-105", "on-200", "on-neg", "on-zero"]),
            ("gt100", &["on-105", "on-200"]),
            ("ge100", &["on-100", "on-105", "on-200"]),
            ("lt100", &["on-neg", "on-zero"]),
            ("le100", &["on-100", "on-neg", "on-zero"]),
            ("lt0", &["on-neg"]),
            // Not a number: must match nothing. It used to be read as 0.
            ("abc", &[]),
            ("apabc", &[]),
            ("neabc", &[]),
            ("gtabc", &[]),
        ];
        let mut failures = Vec::new();
        for (value, expected) in cases {
            let result = backend
                .resolve_chain(
                    &tenant(),
                    "Observation",
                    "has-member:MolecularSequence.window-start",
                    value,
                )
                .await;
            check(&mut failures, "window-start", value, result, expected);
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    #[tokio::test]
    async fn resolve_chain_quantity_terminal() {
        let backend = seeded_backend().await;
        let cases: &[(&str, &[&str])] = &[
            // No unit: the stored number alone is compared.
            ("ap5.4", &["dr-a", "dr-b"]),
            ("ap-5.4", &["dr-neg"]),
            ("5.4", &["dr-a"]),
            ("ne5.4", &["dr-b", "dr-c", "dr-g", "dr-neg"]),
            ("gt5.4", &["dr-b", "dr-c"]),
            ("ge5.4", &["dr-a", "dr-b", "dr-c"]),
            ("lt0", &["dr-neg"]),
            ("sa5.4", &["dr-b", "dr-c"]),
            ("eb5.4", &["dr-g", "dr-neg"]),
            ("le5.4", &["dr-a", "dr-g", "dr-neg"]),
            // With a unit, as the unchained search reads it: the stored unit,
            // or a UCUM equivalent (0.0054 g is 5.4 mg).
            ("5.4|http://unitsofmeasure.org|mg", &["dr-a", "dr-g"]),
            ("5.4||mg", &["dr-a", "dr-g"]),
            (
                "ap5.4|http://unitsofmeasure.org|mg",
                &["dr-a", "dr-b", "dr-g"],
            ),
            ("ap-5.4||mg", &["dr-neg"]),
            ("gt5.4||mg", &["dr-b", "dr-c"]),
            ("5.4||kg", &[]),
            // Not a number: must match nothing. It used to be read as 0.
            ("abc", &[]),
            ("apabc", &[]),
            ("neabc", &[]),
            ("abc||mg", &[]),
        ];
        let mut failures = Vec::new();
        for (value, expected) in cases {
            let result = backend
                .resolve_chain(
                    &tenant(),
                    "DiagnosticReport",
                    "result:Observation.value-quantity",
                    value,
                )
                .await;
            check(&mut failures, "value-quantity", value, result, expected);
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    #[tokio::test]
    async fn resolve_reverse_chain_numeric_terminal() {
        let backend = seeded_backend().await;
        let cases: &[(&str, &str, &str, &[&str])] = &[
            ("Observation", "value-quantity", "ap5.4", &["pq-a", "pq-b"]),
            ("Observation", "value-quantity", "ap-5.4", &["pq-neg"]),
            (
                "Observation",
                "value-quantity",
                "5.4||mg",
                &["pq-a", "pq-g"],
            ),
            ("Observation", "value-quantity", "lt0", &["pq-neg"]),
            ("Observation", "value-quantity", "apabc", &[]),
            ("ChargeItem", "factor-override", "ap2", &["pq-a"]),
            ("ChargeItem", "factor-override", "ap-2", &["pq-neg"]),
            ("ChargeItem", "factor-override", "2", &["pq-a"]),
            ("ChargeItem", "factor-override", "ne2", &["pq-c", "pq-neg"]),
            ("ChargeItem", "factor-override", "lt0", &["pq-neg"]),
            ("ChargeItem", "factor-override", "abc", &[]),
            ("ChargeItem", "factor-override", "apabc", &[]),
        ];
        let mut failures = Vec::new();
        for (source, param, value, expected) in cases {
            let rc = ReverseChainedParameter::terminal(
                *source,
                "subject",
                *param,
                SearchValue::parse(value),
            );
            let result = backend
                .resolve_reverse_chain(&tenant(), "Patient", &rc)
                .await;
            check(
                &mut failures,
                &format!("_has {param}"),
                value,
                result,
                expected,
            );
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }
}
