//! FHIR _filter parameter parser.
//!
//! This module implements parsing and SQL generation for the FHIR _filter parameter
//! as defined in <https://build.fhir.org/search_filter.html>.
//!
//! # Grammar
//!
//! ```text
//! filter        = paramExp / logExp / "(" filter ")"
//! logExp        = filter ("and" / "or") filter
//! paramExp      = paramPath SP compareOp SP compValue
//! compareOp     = "eq" / "ne" / "co" / "sw" / "ew" / "gt" / "lt" / "ge" / "le" / "sa" / "eb" / "ap"
//! compValue     = string (with escaping)
//! paramPath     = paramName
//! ```
//!
//! # Example
//!
//! ```ignore
//! // Simple filter: name equals "Smith"
//! _filter=name eq "Smith"
//!
//! // Combined with AND
//! _filter=name eq "Smith" and birthdate gt 1980-01-01
//!
//! // With OR and parentheses
//! _filter=(status eq active or status eq pending) and category eq urgent
//! ```

use super::query_builder::{SqlFragment, SqlParam};

/// Comparison operators supported by _filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOp {
    /// Equal
    Eq,
    /// Not equal
    Ne,
    /// Contains (string)
    Co,
    /// Starts with (string)
    Sw,
    /// Ends with (string)
    Ew,
    /// Greater than
    Gt,
    /// Less than
    Lt,
    /// Greater than or equal
    Ge,
    /// Less than or equal
    Le,
    /// Starts after (date)
    Sa,
    /// Ends before (date)
    Eb,
    /// Approximately equal
    Ap,
}

impl FilterOp {
    /// Parses a filter operator from a string.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "eq" => Some(FilterOp::Eq),
            "ne" => Some(FilterOp::Ne),
            "co" => Some(FilterOp::Co),
            "sw" => Some(FilterOp::Sw),
            "ew" => Some(FilterOp::Ew),
            "gt" => Some(FilterOp::Gt),
            "lt" => Some(FilterOp::Lt),
            "ge" => Some(FilterOp::Ge),
            "le" => Some(FilterOp::Le),
            "sa" => Some(FilterOp::Sa),
            "eb" => Some(FilterOp::Eb),
            "ap" => Some(FilterOp::Ap),
            _ => None,
        }
    }

    /// Returns the SQL operator for this filter op (when applicable).
    pub fn to_sql_op(&self) -> &'static str {
        match self {
            FilterOp::Eq => "=",
            FilterOp::Ne => "!=",
            FilterOp::Gt | FilterOp::Sa => ">",
            FilterOp::Lt | FilterOp::Eb => "<",
            FilterOp::Ge => ">=",
            FilterOp::Le => "<=",
            // These need special handling (LIKE patterns)
            FilterOp::Co | FilterOp::Sw | FilterOp::Ew | FilterOp::Ap => "LIKE",
        }
    }
}

/// Logical operators for combining filter expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOp {
    /// Logical AND.
    And,
    /// Logical OR.
    Or,
}

/// A parsed filter expression.
#[derive(Debug, Clone)]
pub enum FilterExpr {
    /// A simple comparison: paramName op value
    Comparison {
        /// Search parameter name.
        param: String,
        /// Comparison operator.
        op: FilterOp,
        /// Right-hand value as parsed from the filter source.
        value: String,
    },
    /// Logical combination of expressions
    Logical {
        /// Left-hand sub-expression.
        left: Box<FilterExpr>,
        /// Combining operator.
        op: LogicalOp,
        /// Right-hand sub-expression.
        right: Box<FilterExpr>,
    },
    /// Negation of an expression
    Not(Box<FilterExpr>),
}

/// Filter parsing error.
#[derive(Debug, Clone)]
pub struct FilterParseError {
    /// Human-readable parser failure detail.
    pub message: String,
    /// Byte offset within the input where the failure was detected.
    pub position: usize,
}

impl std::fmt::Display for FilterParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Filter parse error at position {}: {}",
            self.position, self.message
        )
    }
}

impl std::error::Error for FilterParseError {}

/// Parser for FHIR _filter expressions.
pub struct FilterParser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> FilterParser<'a> {
    /// Creates a new filter parser.
    pub fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    /// Parses the entire filter expression.
    pub fn parse(input: &str) -> Result<FilterExpr, FilterParseError> {
        let mut parser = FilterParser::new(input);
        let expr = parser.parse_or_expr()?;
        parser.skip_whitespace();
        if parser.pos < parser.input.len() {
            return Err(FilterParseError {
                message: format!(
                    "Unexpected characters after expression: '{}'",
                    &parser.input[parser.pos..]
                ),
                position: parser.pos,
            });
        }
        Ok(expr)
    }

    /// Skips whitespace characters.
    fn skip_whitespace(&mut self) {
        while self.pos < self.input.len() {
            let c = self.input[self.pos..].chars().next().unwrap();
            if c.is_whitespace() {
                self.pos += c.len_utf8();
            } else {
                break;
            }
        }
    }

    /// Peeks at the next character without consuming it.
    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    /// Consumes and returns the next character.
    fn consume(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    /// Checks if the input starts with the given string (case-insensitive).
    fn starts_with_ci(&self, s: &str) -> bool {
        self.input[self.pos..]
            .to_lowercase()
            .starts_with(&s.to_lowercase())
    }

    /// Parses an OR expression (lowest precedence).
    fn parse_or_expr(&mut self) -> Result<FilterExpr, FilterParseError> {
        let mut left = self.parse_and_expr()?;

        loop {
            self.skip_whitespace();
            if self.starts_with_ci("or") && self.is_word_boundary(2) {
                self.pos += 2;
                self.skip_whitespace();
                let right = self.parse_and_expr()?;
                left = FilterExpr::Logical {
                    left: Box::new(left),
                    op: LogicalOp::Or,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }

        Ok(left)
    }

    /// Parses an AND expression.
    fn parse_and_expr(&mut self) -> Result<FilterExpr, FilterParseError> {
        let mut left = self.parse_not_expr()?;

        loop {
            self.skip_whitespace();
            if self.starts_with_ci("and") && self.is_word_boundary(3) {
                self.pos += 3;
                self.skip_whitespace();
                let right = self.parse_not_expr()?;
                left = FilterExpr::Logical {
                    left: Box::new(left),
                    op: LogicalOp::And,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }

        Ok(left)
    }

    /// Checks if position + offset is at a word boundary.
    fn is_word_boundary(&self, offset: usize) -> bool {
        let next_pos = self.pos + offset;
        if next_pos >= self.input.len() {
            return true;
        }
        let c = self.input[next_pos..].chars().next().unwrap();
        !c.is_alphanumeric() && c != '_'
    }

    /// Parses a NOT expression.
    fn parse_not_expr(&mut self) -> Result<FilterExpr, FilterParseError> {
        self.skip_whitespace();
        if self.starts_with_ci("not") && self.is_word_boundary(3) {
            self.pos += 3;
            self.skip_whitespace();
            let expr = self.parse_primary()?;
            Ok(FilterExpr::Not(Box::new(expr)))
        } else {
            self.parse_primary()
        }
    }

    /// Parses a primary expression (comparison or parenthesized expression).
    fn parse_primary(&mut self) -> Result<FilterExpr, FilterParseError> {
        self.skip_whitespace();

        if self.peek() == Some('(') {
            self.consume(); // consume '('
            let expr = self.parse_or_expr()?;
            self.skip_whitespace();
            if self.peek() != Some(')') {
                return Err(FilterParseError {
                    message: "Expected closing parenthesis".to_string(),
                    position: self.pos,
                });
            }
            self.consume(); // consume ')'
            Ok(expr)
        } else {
            self.parse_comparison()
        }
    }

    /// Parses a comparison expression: paramName op value
    fn parse_comparison(&mut self) -> Result<FilterExpr, FilterParseError> {
        self.skip_whitespace();

        // Parse parameter name
        let param = self.parse_identifier()?;
        if param.is_empty() {
            return Err(FilterParseError {
                message: "Expected parameter name".to_string(),
                position: self.pos,
            });
        }

        self.skip_whitespace();

        // Parse operator
        let op = self.parse_operator()?;

        self.skip_whitespace();

        // Parse value
        let value = self.parse_value()?;

        Ok(FilterExpr::Comparison { param, op, value })
    }

    /// Parses an identifier (parameter name).
    fn parse_identifier(&mut self) -> Result<String, FilterParseError> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == ':' {
                self.consume();
            } else {
                break;
            }
        }
        Ok(self.input[start..self.pos].to_string())
    }

    /// Parses a comparison operator.
    fn parse_operator(&mut self) -> Result<FilterOp, FilterParseError> {
        let start = self.pos;

        // Read up to 2 characters for the operator
        let mut op_str = String::new();
        for _ in 0..2 {
            if let Some(c) = self.peek() {
                if c.is_alphabetic() {
                    op_str.push(c);
                    self.consume();
                } else {
                    break;
                }
            }
        }

        FilterOp::parse(&op_str).ok_or_else(|| FilterParseError {
            message: format!("Unknown operator: '{}'", op_str),
            position: start,
        })
    }

    /// Parses a value (quoted string or unquoted token).
    fn parse_value(&mut self) -> Result<String, FilterParseError> {
        self.skip_whitespace();

        if self.peek() == Some('"') {
            self.parse_quoted_string()
        } else {
            self.parse_unquoted_value()
        }
    }

    /// Parses a quoted string value.
    fn parse_quoted_string(&mut self) -> Result<String, FilterParseError> {
        self.consume(); // consume opening quote
        let mut value = String::new();

        loop {
            match self.peek() {
                Some('"') => {
                    self.consume();
                    break;
                }
                Some('\\') => {
                    self.consume();
                    if let Some(escaped) = self.consume() {
                        match escaped {
                            'n' => value.push('\n'),
                            't' => value.push('\t'),
                            'r' => value.push('\r'),
                            '"' => value.push('"'),
                            '\\' => value.push('\\'),
                            _ => {
                                value.push('\\');
                                value.push(escaped);
                            }
                        }
                    }
                }
                Some(c) => {
                    self.consume();
                    value.push(c);
                }
                None => {
                    return Err(FilterParseError {
                        message: "Unterminated string".to_string(),
                        position: self.pos,
                    });
                }
            }
        }

        Ok(value)
    }

    /// Parses an unquoted value (stops at whitespace or logical operators).
    fn parse_unquoted_value(&mut self) -> Result<String, FilterParseError> {
        let start = self.pos;

        while let Some(c) = self.peek() {
            // Stop at whitespace, parentheses, or if we hit a logical operator
            if c.is_whitespace() || c == '(' || c == ')' {
                break;
            }
            // Check for logical operators
            if self.starts_with_ci(" and ") || self.starts_with_ci(" or ") {
                break;
            }
            self.consume();
        }

        let value = self.input[start..self.pos].to_string();
        if value.is_empty() {
            return Err(FilterParseError {
                message: "Expected value".to_string(),
                position: self.pos,
            });
        }

        Ok(value)
    }
}

/// SQL generator for filter expressions.
pub struct FilterSqlGenerator {
    param_offset: usize,
}

impl FilterSqlGenerator {
    /// Creates a new SQL generator with the given parameter offset.
    pub fn new(param_offset: usize) -> Self {
        Self { param_offset }
    }

    /// Generates SQL for a filter expression.
    ///
    /// Returns a SqlFragment that can be used in a WHERE clause with a subquery
    /// against the search_index table.
    pub fn generate(&mut self, expr: &FilterExpr) -> SqlFragment {
        match expr {
            FilterExpr::Comparison { param, op, value } => {
                self.generate_comparison(param, *op, value)
            }
            FilterExpr::Logical { left, op, right } => {
                let left_sql = self.generate(left);
                let right_sql = self.generate(right);
                match op {
                    LogicalOp::And => left_sql.and(right_sql),
                    LogicalOp::Or => left_sql.or(right_sql),
                }
            }
            FilterExpr::Not(inner) => {
                let inner_sql = self.generate(inner);
                SqlFragment::with_params(format!("NOT ({})", inner_sql.sql), inner_sql.params)
            }
        }
    }

    /// Generates SQL for a comparison expression.
    fn generate_comparison(&mut self, param: &str, op: FilterOp, value: &str) -> SqlFragment {
        self.param_offset += 1;
        let param_num = self.param_offset;

        // Determine the column and condition based on parameter and operator
        let (_column, condition, sql_value) = self.build_condition(param, op, value, param_num);

        SqlFragment::with_params(
            format!(
                "resource_id IN (SELECT resource_id FROM search_index WHERE param_name = '{}' AND {})",
                param, condition
            ),
            vec![SqlParam::string(&sql_value)],
        )
    }

    /// Builds the SQL condition and value for a comparison.
    fn build_condition(
        &self,
        param: &str,
        op: FilterOp,
        value: &str,
        param_num: usize,
    ) -> (&'static str, String, String) {
        // Infer the likely column based on parameter name patterns
        let column = self.infer_column(param);

        match op {
            FilterOp::Eq => (
                column,
                format!("{} = ?{}", column, param_num),
                value.to_string(),
            ),
            FilterOp::Ne => (
                column,
                format!("{} != ?{}", column, param_num),
                value.to_string(),
            ),
            FilterOp::Gt | FilterOp::Sa => (
                column,
                format!("{} > ?{}", column, param_num),
                value.to_string(),
            ),
            FilterOp::Lt | FilterOp::Eb => (
                column,
                format!("{} < ?{}", column, param_num),
                value.to_string(),
            ),
            FilterOp::Ge => (
                column,
                format!("{} >= ?{}", column, param_num),
                value.to_string(),
            ),
            FilterOp::Le => (
                column,
                format!("{} <= ?{}", column, param_num),
                value.to_string(),
            ),
            FilterOp::Co => {
                // Contains - use LIKE with wildcards on both sides
                (
                    column,
                    format!("{} LIKE ?{}", column, param_num),
                    format!("%{}%", Self::escape_like(value)),
                )
            }
            FilterOp::Sw => {
                // Starts with - use LIKE with wildcard at end
                (
                    column,
                    format!("{} LIKE ?{}", column, param_num),
                    format!("{}%", Self::escape_like(value)),
                )
            }
            FilterOp::Ew => {
                // Ends with - use LIKE with wildcard at start
                (
                    column,
                    format!("{} LIKE ?{}", column, param_num),
                    format!("%{}", Self::escape_like(value)),
                )
            }
            FilterOp::Ap => {
                // Approximately equal - for numbers, use a range; for strings, use LIKE
                // Simple implementation: treat as contains for strings
                (
                    column,
                    format!("{} LIKE ?{}", column, param_num),
                    format!("%{}%", Self::escape_like(value)),
                )
            }
        }
    }

    /// Infers the search_index column based on parameter name patterns.
    fn infer_column(&self, param: &str) -> &'static str {
        // Common patterns for parameter types
        match param {
            // Date parameters
            "birthdate" | "date" | "issued" | "effective" | "period" | "authored" | "created"
            | "_lastUpdated" => "value_date",
            // Numeric parameters
            "value-quantity" | "dose-quantity" | "age" => "value_quantity_value",
            // Token parameters (identifiers, codes)
            "identifier" | "code" | "status" | "category" | "type" | "class" | "_tag"
            | "_profile" | "_security" | "gender" => "value_token_code",
            // Reference parameters
            "subject" | "patient" | "encounter" | "performer" | "author" | "organization" => {
                "value_reference"
            }
            // URI parameters
            "url" | "system" => "value_uri",
            // Default to string
            _ => "value_string",
        }
    }

    /// Escapes special characters for LIKE patterns.
    fn escape_like(s: &str) -> String {
        s.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_eq() {
        let expr = FilterParser::parse("name eq Smith").unwrap();
        match expr {
            FilterExpr::Comparison { param, op, value } => {
                assert_eq!(param, "name");
                assert_eq!(op, FilterOp::Eq);
                assert_eq!(value, "Smith");
            }
            _ => panic!("Expected Comparison"),
        }
    }

    #[test]
    fn test_parse_quoted_string() {
        let expr = FilterParser::parse("name eq \"John Smith\"").unwrap();
        match expr {
            FilterExpr::Comparison { param, op, value } => {
                assert_eq!(param, "name");
                assert_eq!(op, FilterOp::Eq);
                assert_eq!(value, "John Smith");
            }
            _ => panic!("Expected Comparison"),
        }
    }

    #[test]
    fn test_parse_and_expression() {
        let expr = FilterParser::parse("name eq Smith and birthdate gt 1980-01-01").unwrap();
        match expr {
            FilterExpr::Logical { left, op, right } => {
                assert_eq!(op, LogicalOp::And);
                match left.as_ref() {
                    FilterExpr::Comparison { param, .. } => assert_eq!(param, "name"),
                    _ => panic!("Expected Comparison on left"),
                }
                match right.as_ref() {
                    FilterExpr::Comparison { param, .. } => assert_eq!(param, "birthdate"),
                    _ => panic!("Expected Comparison on right"),
                }
            }
            _ => panic!("Expected Logical"),
        }
    }

    #[test]
    fn test_parse_or_expression() {
        let expr = FilterParser::parse("status eq active or status eq pending").unwrap();
        match expr {
            FilterExpr::Logical { op, .. } => {
                assert_eq!(op, LogicalOp::Or);
            }
            _ => panic!("Expected Logical"),
        }
    }

    #[test]
    fn test_parse_parentheses() {
        let expr =
            FilterParser::parse("(status eq active or status eq pending) and category eq urgent")
                .unwrap();
        match expr {
            FilterExpr::Logical { left, op, right } => {
                assert_eq!(op, LogicalOp::And);
                // Left should be an OR expression
                match left.as_ref() {
                    FilterExpr::Logical { op, .. } => assert_eq!(*op, LogicalOp::Or),
                    _ => panic!("Expected Logical on left"),
                }
                // Right should be a comparison
                match right.as_ref() {
                    FilterExpr::Comparison { param, .. } => assert_eq!(param, "category"),
                    _ => panic!("Expected Comparison on right"),
                }
            }
            _ => panic!("Expected Logical"),
        }
    }

    #[test]
    fn test_parse_not_expression() {
        let expr = FilterParser::parse("not status eq inactive").unwrap();
        match expr {
            FilterExpr::Not(inner) => match inner.as_ref() {
                FilterExpr::Comparison { param, op, value } => {
                    assert_eq!(param, "status");
                    assert_eq!(*op, FilterOp::Eq);
                    assert_eq!(value, "inactive");
                }
                _ => panic!("Expected Comparison inside Not"),
            },
            _ => panic!("Expected Not"),
        }
    }

    #[test]
    fn test_parse_all_operators() {
        let operators = [
            "eq", "ne", "co", "sw", "ew", "gt", "lt", "ge", "le", "sa", "eb", "ap",
        ];
        for op_str in operators {
            let input = format!("field {} value", op_str);
            let expr = FilterParser::parse(&input).unwrap();
            match expr {
                FilterExpr::Comparison { op, .. } => {
                    assert_eq!(FilterOp::parse(op_str), Some(op));
                }
                _ => panic!("Expected Comparison for operator {}", op_str),
            }
        }
    }

    #[test]
    fn test_generate_sql_simple() {
        let expr = FilterParser::parse("name eq Smith").unwrap();
        let mut generator = FilterSqlGenerator::new(0);
        let sql = generator.generate(&expr);

        assert!(sql.sql.contains("param_name = 'name'"));
        assert!(sql.sql.contains("value_string = ?1"));
        assert_eq!(sql.params.len(), 1);
    }

    #[test]
    fn test_generate_sql_contains() {
        let expr = FilterParser::parse("name co mith").unwrap();
        let mut generator = FilterSqlGenerator::new(0);
        let sql = generator.generate(&expr);

        assert!(sql.sql.contains("LIKE"));
        // Value should have % wildcards
        match &sql.params[0] {
            SqlParam::String(s) => assert_eq!(s, "%mith%"),
            _ => panic!("Expected string param"),
        }
    }

    #[test]
    fn test_generate_sql_and() {
        let expr = FilterParser::parse("name eq Smith and status eq active").unwrap();
        let mut generator = FilterSqlGenerator::new(0);
        let sql = generator.generate(&expr);

        assert!(sql.sql.contains(" AND "));
        assert_eq!(sql.params.len(), 2);
    }

    #[test]
    fn test_generate_sql_or() {
        let expr = FilterParser::parse("status eq active or status eq pending").unwrap();
        let mut generator = FilterSqlGenerator::new(0);
        let sql = generator.generate(&expr);

        assert!(sql.sql.contains(" OR "));
        assert_eq!(sql.params.len(), 2);
    }
}
