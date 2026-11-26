//! Preparing a SQL Query's own SQL to run inside the primary database.
//!
//! A storage backend that speaks SQL (SQLite, PostgreSQL) runs a SQL Query or
//! SQL View Library as **one statement in that database**: every `depends-on`
//! ViewDefinition becomes a common table expression holding the view's
//! compiled SQL, and the Library's SQL reads those expressions by label
//! (#2902). Nothing is copied out of the database first.
//!
//! That puts caller-written SQL in the same statement as the tables that hold
//! every tenant's resources, so this module is the boundary that keeps it
//! from reaching them. [`prepare_native_sql`] parses the SQL and accepts it
//! only when all of the following hold:
//!
//! - it is exactly one statement and every statement nested in it is a query;
//! - every relation it reads is a single-part name that is either a declared
//!   label or a CTE the SQL defines itself, and no CTE is read from a place
//!   where the database would resolve the name to a stored table instead;
//! - it has no `SELECT ... INTO`, no `TABLE name`, no locking clause, and no
//!   `FROM` item other than a table, a subquery, a join, `UNNEST` or an
//!   allow-listed table function;
//! - on PostgreSQL, every function it calls is on an allow-list (PostgreSQL
//!   has functions that read files, run SQL held in a string, change session
//!   settings or take locks), is not schema-qualified, and it uses no custom
//!   `OPERATOR(...)`;
//! - every placeholder is a `:name` the caller declared.
//!
//! The statement that is executed is the **re-serialized syntax tree**, not
//! the text that was received. Whatever the database parses is therefore
//! something this module's parser understood and checked; text the two would
//! read differently cannot carry a relation past the checks.
//!
//! The backends add their own layer below this one: a read-only transaction
//! and statement timeout on PostgreSQL, a read-only authorizer on SQLite.

use std::collections::HashSet;
use std::ops::ControlFlow;

use sqlparser::ast::{
    BinaryOperator, Expr, Ident, ObjectName, Query, SetExpr, Statement, TableFactor, Value, Visit,
    VisitMut, Visitor, VisitorMut, visit_relations,
};
use sqlparser::dialect::{Dialect, PostgreSqlDialect, SQLiteDialect};
use sqlparser::parser::Parser;

use super::{BoundParam, SqlQueryError};

/// Prefix of the relation names the server defines around a Library's SQL.
/// A Library may not define a CTE under it.
pub const RESERVED_RELATION_PREFIX: &str = "__sof_";

/// A SQL database a Library's SQL can run in directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeDialect {
    /// SQLite.
    Sqlite,
    /// PostgreSQL.
    Postgres,
}

impl NativeDialect {
    /// The `dialect` codes of an `application/sql;dialect=...` attachment
    /// written for this database, most specific first. The first is the one
    /// named in messages.
    pub fn library_dialects(self) -> &'static [&'static str] {
        match self {
            NativeDialect::Sqlite => &["sqlite"],
            NativeDialect::Postgres => &["postgresql", "postgres"],
        }
    }

    /// Display name of the database.
    pub fn display_name(self) -> &'static str {
        match self {
            NativeDialect::Sqlite => "SQLite",
            NativeDialect::Postgres => "PostgreSQL",
        }
    }

    fn parser_dialect(self) -> Box<dyn Dialect> {
        match self {
            NativeDialect::Sqlite => Box::new(SQLiteDialect {}),
            NativeDialect::Postgres => Box::new(PostgreSqlDialect {}),
        }
    }

    /// The name a relation or CTE is looked up under: SQLite compares
    /// identifiers without case; PostgreSQL folds an unquoted identifier to
    /// lower case and keeps a quoted one as written.
    pub fn relation_key(self, ident: &Ident) -> String {
        match self {
            NativeDialect::Sqlite => ident.value.to_lowercase(),
            NativeDialect::Postgres if ident.quote_style.is_none() => ident.value.to_lowercase(),
            NativeDialect::Postgres => ident.value.clone(),
        }
    }

    /// The text that stands for bound slot `slot` of a parameter declared
    /// with FHIR type `fhir_type`.
    ///
    /// PostgreSQL infers a parameter's type from where it is used, and the
    /// driver then refuses a value of another type. Every parameter is
    /// therefore sent as text and cast, in the SQL, to the type its
    /// declaration names.
    fn placeholder(self, slot: usize, fhir_type: &str) -> String {
        match self {
            NativeDialect::Sqlite => format!("?{slot}"),
            NativeDialect::Postgres => match fhir_type {
                "boolean" => format!("CAST(CAST(${slot} AS text) AS boolean)"),
                "integer" | "positiveInt" | "unsignedInt" | "integer64" => {
                    format!("CAST(CAST(${slot} AS text) AS bigint)")
                }
                "decimal" => format!("CAST(CAST(${slot} AS text) AS numeric)"),
                _ => format!("CAST(${slot} AS text)"),
            },
        }
    }
}

/// A Library's SQL, checked and re-serialized for one database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeSql {
    /// Whether the SQL's own top-level `WITH` is `WITH RECURSIVE`.
    pub recursive: bool,
    /// The SQL's own top-level CTEs, each as `name AS (...)`, in order.
    pub ctes: Vec<String>,
    /// The lookup keys ([`NativeDialect::relation_key`]) of [`Self::ctes`].
    pub cte_names: Vec<String>,
    /// The query without its top-level `WITH` clause.
    pub body: String,
    /// The whole statement, `WITH` clause included.
    pub statement: String,
    /// Indexes into the caller's bindings, in slot order: slot
    /// `first_slot + i` is bound to `bindings[parameters[i]]`.
    pub parameters: Vec<usize>,
}

fn rejected(message: impl Into<String>) -> SqlQueryError {
    SqlQueryError::Unsupported(message.into())
}

/// Checks `sql` (see the module documentation) and re-serializes it for
/// `dialect`.
///
/// `relations` are the lookup keys of the relations the caller defines for
/// this SQL (its labels). `bindings` are the declared parameters; each
/// distinct `:name` the SQL uses is assigned one slot, numbered from
/// `first_slot`.
pub fn prepare_native_sql(
    sql: &str,
    dialect: NativeDialect,
    relations: &HashSet<String>,
    bindings: &[BoundParam],
    first_slot: usize,
) -> Result<NativeSql, SqlQueryError> {
    let mut statements = Parser::parse_sql(dialect.parser_dialect().as_ref(), sql)
        .map_err(|e| rejected(format!("SQL parse error: {e}")))?;
    if statements.len() != 1 {
        return Err(rejected(format!(
            "exactly one SQL statement is required, got {}",
            statements.len()
        )));
    }
    let Statement::Query(mut query) = statements.remove(0) else {
        return Err(SqlQueryError::NotSelect(
            "only SELECT queries are allowed".to_string(),
        ));
    };

    let mut checker = Checker {
        dialect,
        relations,
        frames: Vec::new(),
        table_function: false,
    };
    if let ControlFlow::Break(error) = Visit::visit(&query, &mut checker) {
        return Err(error);
    }

    let mut binder = Binder {
        dialect,
        bindings,
        first_slot,
        parameters: Vec::new(),
    };
    if let ControlFlow::Break(error) = VisitMut::visit(&mut query, &mut binder) {
        return Err(error);
    }

    let statement = query.to_string();
    let with = query.with.take();
    let body = query.to_string();
    let (recursive, ctes, cte_names) = match with {
        Some(with) => (
            with.recursive,
            with.cte_tables.iter().map(ToString::to_string).collect(),
            with.cte_tables
                .iter()
                .map(|cte| dialect.relation_key(&cte.alias.name))
                .collect(),
        ),
        None => (false, Vec::new(), Vec::new()),
    };
    Ok(NativeSql {
        recursive,
        ctes,
        cte_names,
        body,
        statement,
        parameters: binder.parameters,
    })
}

/// Read-only pass: every rule except placeholders.
struct Checker<'a> {
    dialect: NativeDialect,
    relations: &'a HashSet<String>,
    /// CTE names in scope, one frame per enclosing query.
    frames: Vec<Vec<String>>,
    /// Set between a table function's `FROM` item and its name, which the
    /// visitor reports as a relation right after.
    table_function: bool,
}

impl Checker<'_> {
    fn single_part<'n>(
        &self,
        name: &'n ObjectName,
        what: &str,
    ) -> Result<&'n Ident, SqlQueryError> {
        match name.0.as_slice() {
            [ident] => Ok(ident),
            _ => Err(rejected(format!(
                "{what} '{name}' is qualified; only the labels this Library declares in \
                 relatedArtifact (depends-on) and its own CTEs can be read, by unqualified name"
            ))),
        }
    }

    fn in_scope(&self, key: &str) -> bool {
        self.frames
            .iter()
            .any(|frame| frame.iter().any(|n| n == key))
    }

    fn check_relation(&self, name: &ObjectName) -> Result<(), SqlQueryError> {
        let ident = self.single_part(name, "table")?;
        let key = self.dialect.relation_key(ident);
        if self.relations.contains(&key) || self.in_scope(&key) {
            return Ok(());
        }
        Err(rejected(format!(
            "table '{name}' is not a label this Library declares in relatedArtifact \
             (depends-on), nor a CTE of this query"
        )))
    }

    fn check_function(&self, name: &ObjectName, table_valued: bool) -> Result<(), SqlQueryError> {
        let ident = self.single_part(name, "function")?;
        let lower = ident.value.to_lowercase();
        let allowed = match self.dialect {
            // SQLite's built-in functions cannot read a table or a file.
            // `load_extension` loads code; `pragma_*` table functions read
            // the schema. Both are refused by the authorizer as well.
            NativeDialect::Sqlite => {
                lower != "load_extension"
                    && !(table_valued
                        && (lower.starts_with("pragma_") || lower.starts_with("sqlite_")))
            }
            NativeDialect::Postgres => {
                // A quoted name is taken as written, so only an all-lower-case
                // one can name a function of the list.
                (ident.quote_style.is_none() || ident.value == lower)
                    && if table_valued {
                        POSTGRES_TABLE_FUNCTIONS
                            .binary_search(&lower.as_str())
                            .is_ok()
                    } else {
                        POSTGRES_FUNCTIONS.binary_search(&lower.as_str()).is_ok()
                    }
            }
        };
        if allowed {
            Ok(())
        } else {
            Err(rejected(format!(
                "function '{name}' is not available to SQL Queries on {}",
                self.dialect.display_name()
            )))
        }
    }

    /// The parts of a query body the visitor does not report on their own.
    fn check_body(&self, body: &SetExpr) -> Result<(), SqlQueryError> {
        match body {
            SetExpr::Select(select) => {
                if select.into.is_some() {
                    return Err(SqlQueryError::NotSelect(
                        "only SELECT queries are allowed; SELECT ... INTO is not permitted"
                            .to_string(),
                    ));
                }
                Ok(())
            }
            // A nested query is visited as a query of its own.
            SetExpr::Query(_) | SetExpr::Values(_) => Ok(()),
            SetExpr::SetOperation { left, right, .. } => {
                self.check_body(left)?;
                self.check_body(right)
            }
            SetExpr::Insert(_) | SetExpr::Update(_) => Err(SqlQueryError::NotSelect(
                "only SELECT queries are allowed; INSERT and UPDATE statements are not permitted"
                    .to_string(),
            )),
            SetExpr::Table(_) => Err(rejected(
                "the 'TABLE name' form is not supported; write SELECT * FROM name",
            )),
        }
    }

    /// Refuses a CTE read from where the database would not see it.
    ///
    /// In a `WITH` that is not `RECURSIVE`, PostgreSQL shows a CTE only to the
    /// CTEs after it. A CTE body that reads the name of itself or of a later
    /// sibling would read a stored table of that name instead, so that is
    /// refused unless the name is a label or an enclosing query's CTE, which
    /// is what the database resolves it to.
    fn check_cte_order(&self, query: &Query, names: &[String]) -> Result<(), SqlQueryError> {
        let Some(with) = &query.with else {
            return Ok(());
        };
        if with.recursive {
            return Ok(());
        }
        for (index, cte) in with.cte_tables.iter().enumerate() {
            let later = &names[index..];
            let early = visit_relations(&cte.query, |name: &ObjectName| {
                let [ident] = name.0.as_slice() else {
                    return ControlFlow::Continue(());
                };
                let key = self.dialect.relation_key(ident);
                if later.contains(&key) && !self.relations.contains(&key) && !self.in_scope(&key) {
                    ControlFlow::Break(key)
                } else {
                    ControlFlow::Continue(())
                }
            });
            if let ControlFlow::Break(name) = early {
                return Err(rejected(format!(
                    "CTE '{}' reads '{name}' before '{name}' is defined; reorder the CTEs, \
                     rename one, or use WITH RECURSIVE",
                    cte.alias.name
                )));
            }
        }
        Ok(())
    }
}

impl Visitor for Checker<'_> {
    type Break = SqlQueryError;

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<Self::Break> {
        let mut names = Vec::new();
        if let Some(with) = &query.with {
            for cte in &with.cte_tables {
                let key = self.dialect.relation_key(&cte.alias.name);
                if key.to_lowercase().starts_with(RESERVED_RELATION_PREFIX) {
                    return ControlFlow::Break(rejected(format!(
                        "CTE name '{}' is reserved: names starting with \
                         '{RESERVED_RELATION_PREFIX}' belong to the server",
                        cte.alias.name
                    )));
                }
                names.push(key);
            }
        }
        if let Err(error) = self.check_cte_order(query, &names) {
            return ControlFlow::Break(error);
        }
        if !query.locks.is_empty() {
            return ControlFlow::Break(SqlQueryError::NotSelect(
                "only SELECT queries are allowed; a locking clause (FOR UPDATE / FOR SHARE) is \
                 not permitted"
                    .to_string(),
            ));
        }
        if let Err(error) = self.check_body(&query.body) {
            return ControlFlow::Break(error);
        }
        self.frames.push(names);
        ControlFlow::Continue(())
    }

    fn post_visit_query(&mut self, _query: &Query) -> ControlFlow<Self::Break> {
        self.frames.pop();
        ControlFlow::Continue(())
    }

    fn pre_visit_table_factor(&mut self, factor: &TableFactor) -> ControlFlow<Self::Break> {
        let checked = match factor {
            TableFactor::Table {
                name,
                args: Some(_),
                ..
            } => {
                self.table_function = true;
                self.check_function(name, true)
            }
            // Checked as a relation, right after this.
            TableFactor::Table { args: None, .. } => Ok(()),
            TableFactor::Function { name, .. } => self.check_function(name, true),
            TableFactor::Derived { .. }
            | TableFactor::NestedJoin { .. }
            | TableFactor::UNNEST { .. } => Ok(()),
            other => Err(rejected(format!(
                "FROM item '{other}' is not supported in a SQL Query"
            ))),
        };
        match checked {
            Ok(()) => ControlFlow::Continue(()),
            Err(error) => ControlFlow::Break(error),
        }
    }

    fn pre_visit_relation(&mut self, relation: &ObjectName) -> ControlFlow<Self::Break> {
        if std::mem::take(&mut self.table_function) {
            return ControlFlow::Continue(());
        }
        match self.check_relation(relation) {
            Ok(()) => ControlFlow::Continue(()),
            Err(error) => ControlFlow::Break(error),
        }
    }

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        let checked = match expr {
            Expr::Function(function) => self.check_function(&function.name, false),
            Expr::BinaryOp {
                op: BinaryOperator::PGCustomBinaryOperator(_),
                ..
            } => Err(rejected("the OPERATOR(...) syntax is not supported")),
            _ => Ok(()),
        };
        match checked {
            Ok(()) => ControlFlow::Continue(()),
            Err(error) => ControlFlow::Break(error),
        }
    }
}

/// Rewrites each `:name` placeholder to its bound slot and refuses any other
/// placeholder.
struct Binder<'a> {
    dialect: NativeDialect,
    bindings: &'a [BoundParam],
    first_slot: usize,
    parameters: Vec<usize>,
}

impl VisitorMut for Binder<'_> {
    type Break = SqlQueryError;

    fn pre_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<Self::Break> {
        let Expr::Value(Value::Placeholder(text)) = expr else {
            return ControlFlow::Continue(());
        };
        let Some(name) = text.strip_prefix(':') else {
            return ControlFlow::Break(SqlQueryError::BindParameter(format!(
                "placeholder '{text}' is not supported; use :name with a parameter declared in \
                 Library.parameter"
            )));
        };
        let Some(index) = self.bindings.iter().position(|b| b.name == name) else {
            return ControlFlow::Break(SqlQueryError::BindParameter(format!(
                "placeholder ':{name}' is not declared in Library.parameter"
            )));
        };
        let position = match self.parameters.iter().position(|used| *used == index) {
            Some(position) => position,
            None => {
                self.parameters.push(index);
                self.parameters.len() - 1
            }
        };
        *text = self
            .dialect
            .placeholder(self.first_slot + position, &self.bindings[index].fhir_type);
        ControlFlow::Continue(())
    }
}

/// Functions a SQL Query may call on PostgreSQL. Sorted (binary search).
///
/// Everything here computes a value from its arguments (or is a pure
/// aggregate or window function). Left out on purpose: anything that reads
/// files or large objects, runs SQL held in a string (`query_to_xml`,
/// `dblink`), reads or changes settings (`set_config`, `current_setting`),
/// reports on the server or other sessions, takes locks, sleeps, or touches
/// sequences.
const POSTGRES_FUNCTIONS: &[&str] = &[
    "abs",
    "acos",
    "age",
    "any_value",
    "array_agg",
    "array_append",
    "array_cat",
    "array_length",
    "array_position",
    "array_prepend",
    "array_remove",
    "array_replace",
    "array_to_json",
    "array_to_string",
    "ascii",
    "asin",
    "atan",
    "atan2",
    "avg",
    "bit_and",
    "bit_length",
    "bit_or",
    "bool_and",
    "bool_or",
    "btrim",
    "cardinality",
    "cbrt",
    "ceil",
    "ceiling",
    "char_length",
    "character_length",
    "chr",
    "clock_timestamp",
    "coalesce",
    "concat",
    "concat_ws",
    "corr",
    "cos",
    "cot",
    "count",
    "covar_pop",
    "covar_samp",
    "cume_dist",
    "current_date",
    "current_time",
    "current_timestamp",
    "date",
    "date_bin",
    "date_part",
    "date_trunc",
    "decode",
    "degrees",
    "dense_rank",
    "div",
    "encode",
    "every",
    "exp",
    "extract",
    "first_value",
    "floor",
    "format",
    "gcd",
    "greatest",
    "initcap",
    "isfinite",
    "json_agg",
    "json_array_elements",
    "json_array_elements_text",
    "json_array_length",
    "json_build_array",
    "json_build_object",
    "json_extract_path",
    "json_extract_path_text",
    "json_object_agg",
    "json_typeof",
    "jsonb_agg",
    "jsonb_array_elements",
    "jsonb_array_elements_text",
    "jsonb_array_length",
    "jsonb_build_array",
    "jsonb_build_object",
    "jsonb_extract_path",
    "jsonb_extract_path_text",
    "jsonb_object_agg",
    "jsonb_path_exists",
    "jsonb_path_query_array",
    "jsonb_path_query_first",
    "jsonb_typeof",
    "justify_days",
    "justify_hours",
    "justify_interval",
    "lag",
    "last_value",
    "lcm",
    "lead",
    "least",
    "left",
    "length",
    "ln",
    "localtime",
    "localtimestamp",
    "log",
    "log10",
    "lower",
    "lpad",
    "ltrim",
    "make_date",
    "make_interval",
    "make_time",
    "make_timestamp",
    "make_timestamptz",
    "max",
    "md5",
    "min",
    "mod",
    "mode",
    "now",
    "nth_value",
    "ntile",
    "nullif",
    "num_nonnulls",
    "num_nulls",
    "octet_length",
    "overlay",
    "percent_rank",
    "percentile_cont",
    "percentile_disc",
    "pi",
    "position",
    "power",
    "quote_ident",
    "quote_literal",
    "quote_nullable",
    "radians",
    "random",
    "rank",
    "regexp_count",
    "regexp_instr",
    "regexp_like",
    "regexp_match",
    "regexp_replace",
    "regexp_split_to_array",
    "regexp_substr",
    "regr_avgx",
    "regr_avgy",
    "regr_count",
    "regr_intercept",
    "regr_r2",
    "regr_slope",
    "regr_sxx",
    "regr_sxy",
    "regr_syy",
    "repeat",
    "replace",
    "reverse",
    "right",
    "round",
    "row_number",
    "row_to_json",
    "rpad",
    "rtrim",
    "sha224",
    "sha256",
    "sha384",
    "sha512",
    "sign",
    "sin",
    "split_part",
    "sqrt",
    "starts_with",
    "statement_timestamp",
    "stddev",
    "stddev_pop",
    "stddev_samp",
    "string_agg",
    "string_to_array",
    "strpos",
    "substr",
    "substring",
    "sum",
    "tan",
    "timezone",
    "to_char",
    "to_date",
    "to_hex",
    "to_json",
    "to_jsonb",
    "to_number",
    "to_timestamp",
    "transaction_timestamp",
    "translate",
    "trim",
    "trim_scale",
    "trunc",
    "unnest",
    "upper",
    "var_pop",
    "var_samp",
    "variance",
    "width_bucket",
];

/// Functions a SQL Query may use as a `FROM` item on PostgreSQL. Sorted.
const POSTGRES_TABLE_FUNCTIONS: &[&str] = &[
    "generate_series",
    "json_array_elements",
    "json_array_elements_text",
    "json_each",
    "json_each_text",
    "json_object_keys",
    "jsonb_array_elements",
    "jsonb_array_elements_text",
    "jsonb_each",
    "jsonb_each_text",
    "jsonb_object_keys",
    "jsonb_path_query",
    "regexp_matches",
    "regexp_split_to_table",
    "string_to_table",
    "unnest",
];

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::types::Value as SqlValue;

    fn labels(names: &[&str]) -> HashSet<String> {
        names.iter().map(|n| (*n).to_string()).collect()
    }

    fn binding(name: &str, fhir_type: &str) -> BoundParam {
        BoundParam {
            name: name.to_string(),
            value: SqlValue::Null,
            fhir_type: fhir_type.to_string(),
        }
    }

    fn prepare(sql: &str, dialect: NativeDialect) -> Result<NativeSql, SqlQueryError> {
        prepare_native_sql(
            sql,
            dialect,
            &labels(&["obs", "fp"]),
            &[binding("min_height", "decimal"), binding("city", "string")],
            7,
        )
    }

    fn refused(sql: &str, dialect: NativeDialect) -> String {
        prepare(sql, dialect).expect_err(sql).to_string()
    }

    const BOTH: [NativeDialect; 2] = [NativeDialect::Sqlite, NativeDialect::Postgres];

    #[test]
    fn function_lists_are_sorted_for_binary_search() {
        for list in [POSTGRES_FUNCTIONS, POSTGRES_TABLE_FUNCTIONS] {
            assert!(list.windows(2).all(|pair| pair[0] < pair[1]), "{list:?}");
        }
    }

    #[test]
    fn a_join_over_labels_is_accepted_and_its_parameters_take_slots() {
        let sql = "SELECT fp.id, MAX(obs.value) AS height FROM fp JOIN obs ON obs.patient_id = fp.id \
                   WHERE obs.value > :min_height AND fp.city = :city AND obs.value < :min_height * 2 \
                   GROUP BY fp.id";
        let sqlite = prepare(sql, NativeDialect::Sqlite).unwrap();
        assert_eq!(sqlite.parameters, vec![0, 1]);
        assert!(sqlite.body.contains("obs.value > ?7"), "{}", sqlite.body);
        assert!(sqlite.body.contains("fp.city = ?8"), "{}", sqlite.body);
        assert!(
            sqlite.body.contains("obs.value < ?7 * 2"),
            "{}",
            sqlite.body
        );
        assert!(sqlite.ctes.is_empty() && !sqlite.recursive);

        let pg = prepare(sql, NativeDialect::Postgres).unwrap();
        assert!(
            pg.body
                .contains("obs.value > CAST(CAST($7 AS text) AS numeric)"),
            "{}",
            pg.body
        );
        assert!(
            pg.body.contains("fp.city = CAST($8 AS text)"),
            "{}",
            pg.body
        );
    }

    #[test]
    fn the_querys_own_ctes_are_split_from_its_body() {
        for dialect in BOTH {
            let prepared = prepare(
                "WITH tall AS (SELECT * FROM obs WHERE value > 150), n AS (SELECT count(*) AS c FROM tall) \
                 SELECT c FROM n ORDER BY c",
                dialect,
            )
            .unwrap();
            assert_eq!(prepared.cte_names, vec!["tall", "n"]);
            assert_eq!(prepared.ctes.len(), 2);
            assert!(
                prepared.ctes[0].starts_with("tall AS ("),
                "{:?}",
                prepared.ctes
            );
            assert_eq!(prepared.body, "SELECT c FROM n ORDER BY c");
            assert!(prepared.statement.starts_with("WITH tall AS ("));
        }
    }

    #[test]
    fn a_table_that_is_not_a_label_is_refused_wherever_it_is_named() {
        for dialect in BOTH {
            for sql in [
                "SELECT * FROM resources",
                "SELECT * FROM obs JOIN resources r ON true",
                "SELECT * FROM obs WHERE EXISTS (SELECT 1 FROM resources)",
                "SELECT (SELECT max(id) FROM resources) FROM obs",
                "SELECT * FROM obs UNION ALL SELECT * FROM resources",
                "WITH x AS (SELECT * FROM resources) SELECT * FROM x",
                "SELECT * FROM (SELECT * FROM resources) t",
                "SELECT * FROM obs WHERE id IN (SELECT id FROM resources)",
                "SELECT * FROM obs, LATERAL (SELECT * FROM resources) r",
            ] {
                let message = refused(sql, dialect);
                assert!(
                    message.contains("'resources' is not a label"),
                    "{sql}: {message}"
                );
            }
        }
    }

    #[test]
    fn a_qualified_table_is_refused() {
        for dialect in BOTH {
            for sql in [
                "SELECT * FROM public.resources",
                "SELECT * FROM main.obs",
                "SELECT * FROM pg_catalog.pg_class",
            ] {
                assert!(refused(sql, dialect).contains("is qualified"), "{sql}");
            }
        }
    }

    /// The name a CTE shadows must never be reachable from outside the CTE's
    /// scope, where the database would read the stored table.
    #[test]
    fn a_cte_does_not_make_its_name_readable_outside_its_scope() {
        for dialect in BOTH {
            for sql in [
                // Outer reference; the CTE lives in the subquery only.
                "SELECT * FROM resources WHERE EXISTS (WITH resources AS (SELECT 1 AS a) SELECT 1 FROM resources)",
                // Sibling subquery.
                "SELECT * FROM (WITH resources AS (SELECT 1 AS a) SELECT * FROM resources) x, resources",
            ] {
                let message = refused(sql, dialect);
                assert!(
                    message.contains("'resources' is not a label"),
                    "{sql}: {message}"
                );
            }
            for sql in [
                // An earlier sibling reads the later CTE's name.
                "WITH a AS (SELECT * FROM resources), resources AS (SELECT 1 AS x) SELECT * FROM a",
                // A CTE reads its own name without RECURSIVE.
                "WITH resources AS (SELECT * FROM resources) SELECT * FROM resources",
            ] {
                let message = refused(sql, dialect);
                assert!(
                    message.contains("before 'resources' is defined"),
                    "{sql}: {message}"
                );
            }
        }
    }

    #[test]
    fn a_cte_may_read_earlier_ctes_labels_and_itself_when_recursive() {
        for dialect in BOTH {
            for sql in [
                "WITH a AS (SELECT * FROM obs), b AS (SELECT * FROM a) SELECT * FROM b",
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 3) SELECT i FROM n",
                "SELECT * FROM obs WHERE id IN (WITH t AS (SELECT id FROM fp) SELECT id FROM t)",
            ] {
                prepare(sql, dialect).unwrap_or_else(|e| panic!("{sql}: {e}"));
            }
        }
        assert!(
            prepare(
                "WITH RECURSIVE n(i) AS (SELECT 1) SELECT i FROM n",
                NativeDialect::Postgres
            )
            .unwrap()
            .recursive
        );
    }

    #[test]
    fn statements_that_are_not_plain_queries_are_refused() {
        for dialect in BOTH {
            for sql in [
                "DELETE FROM obs",
                "SELECT 1; SELECT 2",
                "WITH x AS (SELECT 1) INSERT INTO obs SELECT * FROM x",
                "SELECT * INTO copy FROM obs",
                "SELECT * FROM obs FOR UPDATE",
            ] {
                assert!(prepare(sql, dialect).is_err(), "{sql}");
            }
        }
        assert!(prepare("TABLE obs", NativeDialect::Postgres).is_err());
    }

    #[test]
    fn postgres_functions_outside_the_list_are_refused() {
        for sql in [
            "SELECT pg_read_file('/etc/passwd')",
            "SELECT query_to_xml('select * from resources', true, false, '')",
            "SELECT set_config('statement_timeout', '0', true)",
            "SELECT current_setting('server_version')",
            "SELECT pg_sleep(100)",
            "SELECT pg_advisory_lock(1)",
            "SELECT pg_catalog.lower('A')",
            "SELECT lo_import('/etc/passwd')",
            "SELECT * FROM dblink('x', 'select 1') AS t(a int)",
            "SELECT * FROM pg_ls_dir('.')",
            "SELECT current_user",
            "SELECT \"Lower\"('A')",
            "SELECT count(*) FILTER (WHERE pg_sleep(1) IS NULL) FROM obs",
            "SELECT * FROM obs ORDER BY pg_sleep(1)",
        ] {
            let message = refused(sql, NativeDialect::Postgres);
            assert!(
                message.contains("is not available") || message.contains("qualified"),
                "{sql}: {message}"
            );
        }
        assert!(
            refused("SELECT 1 OPERATOR(pg_catalog.+) 2", NativeDialect::Postgres)
                .contains("OPERATOR")
        );
    }

    #[test]
    fn ordinary_postgres_sql_is_accepted() {
        for sql in [
            "SELECT lower(code), count(*), round(avg(value), 2), string_agg(id, ',' ORDER BY id) FROM obs GROUP BY 1",
            "SELECT id, row_number() OVER (PARTITION BY patient_id ORDER BY value DESC) FROM obs",
            "SELECT date_trunc('month', effective::timestamptz), coalesce(value, 0)::int FROM obs",
            "SELECT extract(year FROM effective::date), substring(code FROM 1 FOR 3), trim(code) FROM obs",
            "SELECT g FROM generate_series(1, 3) AS g",
            "SELECT * FROM obs CROSS JOIN LATERAL unnest(string_to_array(code, '-')) AS part",
            "SELECT current_date, now(), CASE WHEN value > 1 THEN 'a' ELSE 'b' END FROM obs",
            "SELECT \"patientId\" FROM obs WHERE code ILIKE 'a%' AND value BETWEEN 1 AND 2",
        ] {
            prepare(sql, NativeDialect::Postgres).unwrap_or_else(|e| panic!("{sql}: {e}"));
        }
    }

    #[test]
    fn sqlite_refuses_extension_loading_and_schema_table_functions() {
        for sql in [
            "SELECT load_extension('x')",
            "SELECT * FROM pragma_table_info('resources')",
            "SELECT * FROM sqlite_master",
        ] {
            assert!(prepare(sql, NativeDialect::Sqlite).is_err(), "{sql}");
        }
        for sql in [
            "SELECT j.value FROM obs, json_each(obs.codes) AS j",
            "SELECT strftime('%Y', effective), substr(code, 1, 3), ifnull(value, 0) FROM obs",
        ] {
            prepare(sql, NativeDialect::Sqlite).unwrap_or_else(|e| panic!("{sql}: {e}"));
        }
    }

    #[test]
    fn only_declared_colon_placeholders_are_bound() {
        for dialect in BOTH {
            assert!(
                matches!(
                    prepare("SELECT * FROM obs WHERE value > :unknown", dialect),
                    Err(SqlQueryError::BindParameter(_))
                ),
                "{dialect:?}"
            );
            // Positional placeholders would address the server's own slots.
            for sql in [
                "SELECT * FROM obs WHERE value > ?",
                "SELECT * FROM obs WHERE value > $1",
            ] {
                assert!(prepare(sql, dialect).is_err(), "{dialect:?}: {sql}");
            }
        }
    }

    #[test]
    fn server_relation_names_cannot_be_defined_or_read() {
        for dialect in BOTH {
            assert!(
                refused(
                    "WITH __sof_node_0 AS (SELECT 1) SELECT * FROM __sof_node_0",
                    dialect
                )
                .contains("reserved")
            );
            assert!(refused("SELECT * FROM __sof_node_0", dialect).contains("is not a label"));
        }
    }

    #[test]
    fn postgres_folds_unquoted_names_and_keeps_quoted_ones() {
        prepare("SELECT * FROM OBS", NativeDialect::Postgres).unwrap();
        prepare("SELECT * FROM \"obs\"", NativeDialect::Postgres).unwrap();
        assert!(prepare("SELECT * FROM \"OBS\"", NativeDialect::Postgres).is_err());
        prepare("SELECT * FROM \"OBS\"", NativeDialect::Sqlite).unwrap();
    }
}
