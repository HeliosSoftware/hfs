//! Chain Query Builder for FHIR Search.
//!
//! Generates efficient SQL subqueries for:
//! - Forward chained parameters (e.g., `Observation?subject.organization.name=Hospital`)
//! - Reverse chained parameters (_has) (e.g., `Patient?_has:Observation:subject:code=1234-5`)
//!
//! Uses the search_index table to resolve chains efficiently via SQL subqueries
//! instead of in-memory iteration.

// Error enum variant fields are self-documenting
#![allow(missing_docs)]

use std::sync::Arc;

use parking_lot::RwLock;

use crate::error::{BackendError, StorageResult};
use crate::search::SearchParameterRegistry;
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
    MaxDepthExceeded { depth: usize, max: usize },
    /// Reference parameter not found in registry.
    UnknownReferenceParam {
        resource_type: String,
        param: String,
    },
    /// Cannot determine target type for reference.
    AmbiguousTargetType {
        resource_type: String,
        param: String,
    },
    /// Terminal parameter not found.
    UnknownTerminalParam {
        resource_type: String,
        param: String,
    },
    /// Chain is empty.
    EmptyChain,
    /// Invalid chain syntax.
    InvalidSyntax { message: String },
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
                    return Ok(self.infer_target_type(ref_param));
                } else {
                    // Multiple targets - use inference for common patterns
                    // This allows queries like `Observation?subject.name=Smith` to work
                    // by defaulting `subject` to `Patient`
                    return Ok(self.infer_target_type(ref_param));
                }
            }
        }

        // Fall back to inference based on common parameter names
        Ok(self.infer_target_type(ref_param))
    }

    /// Infers target type based on common parameter naming conventions.
    fn infer_target_type(&self, ref_param: &str) -> String {
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
                // Default: capitalize first letter
                let mut chars = ref_param.chars();
                match chars.next() {
                    Some(c) => c.to_uppercase().chain(chars).collect(),
                    None => ref_param.to_string(),
                }
            }
        }
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
        let (terminal_sql, terminal_param) =
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

        Ok(SqlFragment::with_params(final_sql, vec![terminal_param]))
    }

    /// Builds the terminal condition for a chain query.
    fn build_terminal_condition(
        &self,
        chain: &ParsedChain,
        value: &SearchValue,
        param_num: usize,
    ) -> StorageResult<(String, SqlParam)> {
        let alias_num = chain.links.len();
        let alias = format!("si{}", alias_num);

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
                                "({}.value_token_system IS NULL OR {}.value_token_system = '') \
                                 AND {}.value_token_code = ?{}",
                                alias, alias, alias, param_num
                            ),
                            SqlParam::String(code.to_string()),
                        )
                    } else {
                        (
                            format!(
                                "{}.value_token_system = '{}' AND {}.value_token_code = ?{}",
                                alias,
                                system.replace('\'', "''"),
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
                let num_col = format!("{}.value_number", alias);
                build_number_condition(&num_col, value, param_num)
            }
            SearchParamType::Quantity => {
                // Quantity comparison on value_quantity_value
                let qty_col = format!("{}.value_quantity_value", alias);
                build_number_condition(&qty_col, value, param_num)
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

        Ok((condition, param))
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
            let (search_condition, search_param) = self.build_reverse_terminal_condition(
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

            Ok((sql, vec![search_param]))
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
    ) -> StorageResult<(String, SqlParam)> {
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
                                "({}.value_token_system IS NULL OR {}.value_token_system = '') \
                                 AND {}.value_token_code = ?{}",
                                alias, alias, alias, param_num
                            ),
                            SqlParam::String(code.to_string()),
                        )
                    } else {
                        (
                            format!(
                                "{}.value_token_system = '{}' AND {}.value_token_code = ?{}",
                                alias,
                                system.replace('\'', "''"),
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
                let num_col = format!("{}.value_number", alias);
                build_number_condition(&num_col, value, param_num)
            }
            SearchParamType::Quantity => {
                let qty_col = format!("{}.value_quantity_value", alias);
                build_number_condition(&qty_col, value, param_num)
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

        Ok((condition, param))
    }
}

/// Builds a date comparison condition.
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
            // Approximately equal: within a day for dates
            return (
                format!("DATE({}) = DATE(?{})", column, param_num),
                SqlParam::String(value.value.clone()),
            );
        }
    };

    (
        format!("{} {} ?{}", column, op, param_num),
        SqlParam::String(val.clone()),
    )
}

/// Builds a number comparison condition.
fn build_number_condition(
    column: &str,
    value: &SearchValue,
    param_num: usize,
) -> (String, SqlParam) {
    use crate::types::SearchPrefix;

    // Try to parse as a number
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
            // Approximately equal: within 10% for numbers
            let lower = num_value * 0.9;
            let upper = num_value * 1.1;
            return (
                format!("{} BETWEEN {} AND {}", column, lower, upper),
                SqlParam::Float(num_value),
            );
        }
    };

    (
        format!("{} {} ?{}", column, op, param_num),
        SqlParam::Float(val),
    )
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
