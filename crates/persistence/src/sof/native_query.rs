//! Composing a SQL Query Library and its dependency graph into one statement
//! for the primary database (#2902).
//!
//! The statement has this shape, with every relation a common table
//! expression:
//!
//! ```sql
//! WITH
//!   "__sof_raw_0"("c0", "c1") AS (<the view's compiled SQL>),
//!   "__sof_node_0" AS (SELECT <typed c0> AS "id", <typed c1> AS "value" FROM "__sof_raw_0"),
//!   "obs" AS (SELECT * FROM "__sof_node_0"),      -- one per label
//!   "__sof_node_1" AS (<a SQL View's SQL>),
//!   <the Library's own CTEs>
//! <the Library's own query>
//! ```
//!
//! Nothing is read out of the database before the Library's SQL runs: the
//! database plans the views and the query together, so a filter, join or
//! aggregate of the query is applied while the resources are scanned.
//!
//! The compiled view SQL returns every column as text in a wire form the row
//! readers decode. A query needs values it can compare and add, so the
//! `__sof_node_N` projection turns each column into the SQL type its
//! ViewDefinition declares: `boolean`, an integer or `decimal` as such,
//! everything else (dates included: a FHIR date may be partial) as text.
//!
//! Caller-written SQL enters only through
//! [`helios_sof::sqlquery::prepare_native_sql`], which is the check that keeps
//! it from naming anything but these relations.

use std::collections::HashSet;
use std::future::Future;

use helios_fhir::FhirVersion;
use helios_sof::sqlquery::{
    ColumnFhirType, NativeDialect, RESERVED_RELATION_PREFIX, SqlQueryError, SqlValue, TableSchema,
    prepare_native_sql,
};
use serde_json::Value;

use crate::core::sof_runner::{
    NativeRelation, NativeRelationSource, NativeSqlQuery, SofError, ViewFilters,
};

use super::compiler::SqlDialect;
use super::decode::ColumnDecode;
use super::emit::strip_trailing_order_by;
use super::runtime::{RuntimeParam, prepare_sql_run};

/// Stored tables the compiled view SQL reads. A label or a CTE of the same
/// name, defined beside the view in one `WITH` list, would be read in the
/// table's place (SQLite shows a view every sibling CTE; so does PostgreSQL
/// under `WITH RECURSIVE`).
const STORED_TABLES: &[&str] = &["resources", "search_index"];

/// One bound slot of the composed statement, in slot order.
#[derive(Debug, Clone)]
pub(super) enum NativeParam {
    /// A slot of a compiled view (tenant, resource type, constant, filter).
    Runtime(RuntimeParam),
    /// A `Library.parameter` value.
    User(SqlValue),
    /// The rows of a relation the caller supplied, as one JSON array.
    Json(String),
}

/// The statement and its bound slots.
#[derive(Debug)]
pub(super) struct ComposedQuery {
    pub sql: String,
    pub params: Vec<NativeParam>,
}

fn native_dialect(dialect: SqlDialect) -> NativeDialect {
    match dialect {
        SqlDialect::Sqlite => NativeDialect::Sqlite,
        SqlDialect::Postgres => NativeDialect::Postgres,
    }
}

/// A double-quoted identifier. Labels follow the `sql-name` invariant and
/// column names the SoF column-name rule, so a name with a quote in it is
/// malformed input, refused instead of escaped.
fn quoted(name: &str) -> Result<String, SqlQueryError> {
    if name.is_empty() || name.contains('"') || name.contains('\0') {
        return Err(SqlQueryError::InvalidIdentifier(name.to_string()));
    }
    Ok(format!("\"{name}\""))
}

/// How a failure to compile or prepare a dependency view is reported: a view
/// the backend cannot run is the Library's problem (`422`), not a server
/// fault.
fn view_error(display: &str, error: SofError) -> SqlQueryError {
    match error {
        SofError::Uncompilable { .. } | SofError::InvalidViewDefinition(_) => {
            SqlQueryError::MalformedLibrary(format!("dependency view '{display}': {error}"))
        }
        SofError::Unavailable(message) => SqlQueryError::SourceUnavailable(message),
        other => SqlQueryError::SourceStream(format!("dependency view '{display}': {other}")),
    }
}

/// Adds `offset` to every bound-slot reference (`?N` on SQLite, `$N` on
/// PostgreSQL) of compiled view SQL, so several views can share one statement.
///
/// The compiled SQL numbers its slots from 1. Text inside a string literal, a
/// quoted identifier or a comment is copied unchanged: a JSON path such as
/// `'$.name'` or `'$[0]'` is data, not a slot.
pub(super) fn shift_slots(sql: &str, dialect: SqlDialect, offset: usize) -> String {
    let marker = match dialect {
        SqlDialect::Sqlite => '?',
        SqlDialect::Postgres => '$',
    };
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len() + 16);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' | '"' => {
                // PostgreSQL `E'...'` strings escape with a backslash.
                let escapes = c == '\''
                    && dialect == SqlDialect::Postgres
                    && i > 0
                    && matches!(chars[i - 1], 'E' | 'e')
                    && (i < 2 || !(chars[i - 2].is_alphanumeric() || chars[i - 2] == '_'));
                out.push(c);
                i += 1;
                while i < chars.len() {
                    let d = chars[i];
                    out.push(d);
                    i += 1;
                    if escapes && d == '\\' && i < chars.len() {
                        out.push(chars[i]);
                        i += 1;
                    } else if d == c {
                        // A doubled quote stays inside the literal.
                        if i < chars.len() && chars[i] == c {
                            out.push(c);
                            i += 1;
                        } else {
                            break;
                        }
                    }
                }
            }
            '-' if chars.get(i + 1) == Some(&'-') => {
                while i < chars.len() && chars[i] != '\n' {
                    out.push(chars[i]);
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                while i < chars.len() {
                    if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        out.push_str("*/");
                        i += 2;
                        break;
                    }
                    out.push(chars[i]);
                    i += 1;
                }
            }
            c if c == marker && chars.get(i + 1).is_some_and(char::is_ascii_digit) => {
                let mut j = i + 1;
                let mut n = 0usize;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    n = n * 10 + chars[j].to_digit(10).expect("ascii digit") as usize;
                    j += 1;
                }
                out.push(marker);
                out.push_str(&(n + offset).to_string());
                i = j;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    out
}

/// The SQL type a declared column has inside a query, or `None` for text.
fn sql_type(dialect: SqlDialect, fhir_type: &ColumnFhirType) -> Option<&'static str> {
    match (dialect, fhir_type) {
        (SqlDialect::Postgres, ColumnFhirType::Boolean) => Some("boolean"),
        (SqlDialect::Postgres, ColumnFhirType::Integer | ColumnFhirType::Integer64) => {
            Some("bigint")
        }
        (SqlDialect::Postgres, ColumnFhirType::Decimal) => Some("numeric"),
        (SqlDialect::Sqlite, ColumnFhirType::Boolean) => Some("INTEGER"),
        (SqlDialect::Sqlite, ColumnFhirType::Integer | ColumnFhirType::Integer64) => {
            Some("NUMERIC")
        }
        (SqlDialect::Sqlite, ColumnFhirType::Decimal) => Some("REAL"),
        _ => None,
    }
}

/// Turns raw column `raw` of a compiled view (see the module documentation)
/// into the value a query sees for a column declared as `fhir_type`.
fn typed_column(
    dialect: SqlDialect,
    raw: &str,
    fhir_type: &ColumnFhirType,
    decode: ColumnDecode,
) -> String {
    match dialect {
        SqlDialect::Postgres => match sql_type(dialect, fhir_type) {
            Some(ty) => format!("CAST({raw} AS {ty})"),
            // The JSON form of a value: a string loses its quotes, an array
            // or object stays JSON text.
            None if decode == ColumnDecode::Json => format!(
                "CASE jsonb_typeof(CAST({raw} AS jsonb)) \
                 WHEN 'string' THEN CAST({raw} AS jsonb) #>> '{{}}' \
                 WHEN 'null' THEN NULL ELSE {raw} END"
            ),
            None => raw.to_string(),
        },
        SqlDialect::Sqlite => match fhir_type {
            // `json_extract` yields 1/0 for a JSON boolean; a computed
            // boolean arrives as the text `true` / `false`.
            ColumnFhirType::Boolean => {
                format!("CASE {raw} WHEN 'true' THEN 1 WHEN 'false' THEN 0 ELSE {raw} END")
            }
            ColumnFhirType::Integer | ColumnFhirType::Integer64 => {
                format!("CASE typeof({raw}) WHEN 'text' THEN CAST({raw} AS NUMERIC) ELSE {raw} END")
            }
            ColumnFhirType::Decimal => format!("CAST({raw} AS REAL)"),
            _ if decode == ColumnDecode::Json => format!(
                "CASE WHEN typeof({raw}) = 'text' AND json_valid({raw}) THEN \
                 CASE json_type({raw}) WHEN 'text' THEN json_extract({raw}, '$') \
                 WHEN 'null' THEN NULL ELSE {raw} END \
                 ELSE CAST({raw} AS TEXT) END"
            ),
            _ => format!("CAST({raw} AS TEXT)"),
        },
    }
}

/// A relation with the view's columns and no rows: what a view restricted to
/// a `group` without members yields.
fn empty_view(dialect: SqlDialect, schema: &TableSchema) -> Result<String, SqlQueryError> {
    let mut columns = Vec::with_capacity(schema.columns.len());
    for column in &schema.columns {
        let name = quoted(&column.name)?;
        let ty = sql_type(dialect, &column.fhir_type).unwrap_or(match dialect {
            SqlDialect::Postgres => "text",
            SqlDialect::Sqlite => "TEXT",
        });
        columns.push(format!("CAST(NULL AS {ty}) AS {name}"));
    }
    Ok(format!("SELECT {} WHERE 1 = 0", columns.join(", ")))
}

/// The lookup keys ([`NativeDialect::relation_key`]) a label is defined
/// under. PostgreSQL folds an unquoted name to lower case, so a label with
/// upper-case letters is defined twice: as written (for `"Obs"`) and folded
/// (for `Obs`, `obs`, `OBS`).
fn label_keys(dialect: SqlDialect, label: &str) -> Vec<String> {
    match dialect {
        SqlDialect::Sqlite => vec![label.to_lowercase()],
        SqlDialect::Postgres => {
            let folded = label.to_lowercase();
            if folded == label {
                vec![folded]
            } else {
                vec![folded, label.to_string()]
            }
        }
    }
}

struct Composer<'a> {
    dialect: SqlDialect,
    fhir_version: FhirVersion,
    tenant_id: &'a str,
    filters: ViewFilters,
    ctes: Vec<String>,
    params: Vec<NativeParam>,
    /// Lookup keys of every label defined so far, and of all labels of the
    /// query (what its SQL may name).
    labels: HashSet<String>,
}

impl Composer<'_> {
    async fn add_view<F, Fut>(
        &mut self,
        index: usize,
        relation: &NativeRelation,
        view: &Value,
        load_groups: &F,
    ) -> Result<(), SqlQueryError>
    where
        F: Fn(Vec<String>) -> Fut,
        Fut: Future<Output = Result<Vec<Value>, SofError>>,
    {
        let display = relation.display.as_str();
        let schema = TableSchema::try_from_view_definition(view).map_err(|e| {
            SqlQueryError::MalformedLibrary(format!("dependency view '{display}': {e}"))
        })?;
        let node = quoted(&relation.name)?;
        let prepared = prepare_sql_run(
            view,
            self.dialect,
            self.fhir_version,
            self.tenant_id,
            self.filters.clone(),
            load_groups,
        )
        .await
        .map_err(|e| view_error(display, e))?;
        let Some(prepared) = prepared else {
            self.ctes.push(format!(
                "{node} AS ({})",
                empty_view(self.dialect, &schema)?
            ));
            return Ok(());
        };
        let compiled = prepared.query;
        if compiled.columns.is_empty() {
            return Err(SqlQueryError::MalformedLibrary(format!(
                "dependency view '{display}' selects no columns"
            )));
        }
        // No query can observe the view's own row order, and sorting every
        // row of it before the query's filter would be the expensive part.
        let view_sql = shift_slots(
            strip_trailing_order_by(&compiled.sql),
            self.dialect,
            self.params.len(),
        );
        self.params
            .extend(prepared.params.into_iter().map(NativeParam::Runtime));

        let raw = format!("\"{RESERVED_RELATION_PREFIX}raw_{index}\"");
        let raw_columns: Vec<String> = (0..compiled.columns.len())
            .map(|i| format!("\"c{i}\""))
            .collect();
        // A name declared twice is one column, the first declaration, as in
        // the row runners' JSON objects.
        let mut seen = HashSet::new();
        let mut projection = Vec::with_capacity(compiled.columns.len());
        for (i, name) in compiled.columns.iter().enumerate() {
            if !seen.insert(name.as_str()) {
                continue;
            }
            let fhir_type = schema
                .columns
                .iter()
                .find(|c| &c.name == name)
                .map(|c| c.fhir_type.clone())
                .unwrap_or_else(|| ColumnFhirType::String("string".to_string()));
            let decode = compiled.column_decodes.get(i).copied().unwrap_or_default();
            projection.push(format!(
                "{} AS {}",
                typed_column(self.dialect, &raw_columns[i], &fhir_type, decode),
                quoted(name)?
            ));
        }
        self.ctes.push(format!(
            "{raw}({}) AS (\n{view_sql}\n)",
            raw_columns.join(", ")
        ));
        self.ctes.push(format!(
            "{node} AS (SELECT {} FROM {raw})",
            projection.join(", ")
        ));
        Ok(())
    }

    fn add_sql_view(&mut self, relation: &NativeRelation, sql: &str) -> Result<(), SqlQueryError> {
        let prepared = prepare_native_sql(
            sql,
            native_dialect(self.dialect),
            &self.labels,
            &[],
            self.params.len() + 1,
        )
        .map_err(|e| in_sql_view(&relation.display, e))?;
        self.ctes.push(format!(
            "{} AS (\n{}\n)",
            quoted(&relation.name)?,
            prepared.statement
        ));
        Ok(())
    }

    fn add_rows(
        &mut self,
        relation: &NativeRelation,
        columns: &[(String, String)],
        rows: &[Value],
    ) -> Result<(), SqlQueryError> {
        for (name, _) in columns {
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(SqlQueryError::InvalidIdentifier(name.clone()));
            }
        }
        let json = serde_json::to_string(rows)
            .map_err(|e| SqlQueryError::Internal(format!("relation rows: {e}")))?;
        self.params.push(NativeParam::Json(json));
        let slot = self.params.len();
        let node = quoted(&relation.name)?;
        let body = match self.dialect {
            SqlDialect::Sqlite => {
                let projection: Vec<String> = columns
                    .iter()
                    .map(|(name, _)| format!("json_extract(j.value, '$.{name}') AS \"{name}\""))
                    .collect();
                format!(
                    "SELECT {} FROM json_each(?{slot}) AS j",
                    projection.join(", ")
                )
            }
            SqlDialect::Postgres => {
                let definition: Vec<String> = columns
                    .iter()
                    .map(|(name, fhir_type)| {
                        let ty = sql_type(self.dialect, &ColumnFhirType::from_code(fhir_type))
                            .unwrap_or("text");
                        format!("\"{name}\" {ty}")
                    })
                    .collect();
                format!(
                    "SELECT * FROM jsonb_to_recordset(CAST(CAST(${slot} AS text) AS jsonb)) AS t({})",
                    definition.join(", ")
                )
            }
        };
        self.ctes.push(format!("{node} AS ({body})"));
        Ok(())
    }

    fn add_labels(&mut self, relation: &NativeRelation) -> Result<(), SqlQueryError> {
        let node = quoted(&relation.name)?;
        for label in &relation.labels {
            for key in label_keys(self.dialect, label) {
                self.ctes
                    .push(format!("{} AS (SELECT * FROM {node})", quoted(&key)?));
            }
        }
        Ok(())
    }
}

fn in_sql_view(display: &str, error: SqlQueryError) -> SqlQueryError {
    match error {
        SqlQueryError::Unsupported(message) => {
            SqlQueryError::Unsupported(format!("SQL View '{display}': {message}"))
        }
        SqlQueryError::NotSelect(message) => {
            SqlQueryError::NotSelect(format!("SQL View '{display}': {message}"))
        }
        SqlQueryError::BindParameter(message) => {
            SqlQueryError::BindParameter(format!("SQL View '{display}': {message}"))
        }
        other => other,
    }
}

/// Builds the one statement that runs `query` in a `dialect` database for
/// `tenant_id`. `load_groups` fetches the Group documents a `group` filter
/// names.
pub(super) async fn compose<F, Fut>(
    dialect: SqlDialect,
    fhir_version: FhirVersion,
    tenant_id: &str,
    query: &NativeSqlQuery,
    load_groups: F,
) -> Result<ComposedQuery, SqlQueryError>
where
    F: Fn(Vec<String>) -> Fut,
    Fut: Future<Output = Result<Vec<Value>, SofError>>,
{
    let mut labels = HashSet::new();
    for relation in &query.relations {
        for label in &relation.labels {
            for key in label_keys(dialect, label) {
                if STORED_TABLES.contains(&key.to_lowercase().as_str())
                    || key.to_lowercase().starts_with(RESERVED_RELATION_PREFIX)
                {
                    return Err(SqlQueryError::MalformedLibrary(format!(
                        "relatedArtifact.label '{label}' is reserved on this server; use another \
                         label"
                    )));
                }
                labels.insert(key);
            }
        }
    }
    let mut composer = Composer {
        dialect,
        fhir_version,
        tenant_id,
        // `_limit` caps the query's result, never a dependency.
        filters: ViewFilters {
            limit: None,
            ..query.filters.clone()
        },
        ctes: Vec::new(),
        params: Vec::new(),
        labels,
    };
    for (index, relation) in query.relations.iter().enumerate() {
        match &relation.source {
            NativeRelationSource::View(view) => {
                composer
                    .add_view(index, relation, view, &load_groups)
                    .await?
            }
            NativeRelationSource::Sql(sql) => composer.add_sql_view(relation, sql)?,
            NativeRelationSource::Rows { columns, rows } => {
                composer.add_rows(relation, columns, rows)?
            }
        }
        composer.add_labels(relation)?;
    }

    let subject = prepare_native_sql(
        &query.sql,
        native_dialect(dialect),
        &composer.labels,
        &query.bindings,
        composer.params.len() + 1,
    )?;
    for name in &subject.cte_names {
        let lower = name.to_lowercase();
        if composer.labels.contains(name) {
            return Err(SqlQueryError::Unsupported(format!(
                "CTE '{name}' has the name of a relatedArtifact (depends-on) label; rename the CTE"
            )));
        }
        if STORED_TABLES.contains(&lower.as_str()) {
            return Err(SqlQueryError::Unsupported(format!(
                "CTE name '{name}' is reserved on this server; rename the CTE"
            )));
        }
    }
    for index in &subject.parameters {
        composer
            .params
            .push(NativeParam::User(query.bindings[*index].value.clone()));
    }

    let mut ctes = composer.ctes;
    ctes.extend(subject.ctes);
    let sql = if ctes.is_empty() {
        subject.body
    } else {
        format!(
            "WITH {}{}\n{}",
            if subject.recursive { "RECURSIVE " } else { "" },
            ctes.join(",\n"),
            subject.body
        )
    };
    Ok(ComposedQuery {
        sql,
        params: composer.params,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_are_shifted_outside_literals_identifiers_and_comments() {
        let sqlite = "SELECT json_extract(d, '$.a?1'), \"c?1\" FROM t WHERE x = ?1 AND y = ?12 -- ?1\n AND z = ?2 /* ?3 */";
        assert_eq!(
            shift_slots(sqlite, SqlDialect::Sqlite, 10),
            "SELECT json_extract(d, '$.a?1'), \"c?1\" FROM t WHERE x = ?11 AND y = ?22 -- ?1\n AND z = ?12 /* ?3 */"
        );
        let pg = "SELECT d #>> '{a,$1}', 'it''s $1', E'a\\'$1' FROM t WHERE x = $1 AND y = ANY($3::text[]) AND p @? '$[0]'";
        assert_eq!(
            shift_slots(pg, SqlDialect::Postgres, 4),
            "SELECT d #>> '{a,$1}', 'it''s $1', E'a\\'$1' FROM t WHERE x = $5 AND y = ANY($7::text[]) AND p @? '$[0]'"
        );
        // The other dialect's marker is not a slot.
        assert_eq!(shift_slots("a ?1 $1", SqlDialect::Sqlite, 1), "a ?2 $1");
        assert_eq!(shift_slots("a ?1 $1", SqlDialect::Postgres, 1), "a ?1 $2");
    }

    #[test]
    fn postgres_defines_a_mixed_case_label_folded_and_as_written() {
        assert_eq!(label_keys(SqlDialect::Postgres, "Obs"), vec!["obs", "Obs"]);
        assert_eq!(label_keys(SqlDialect::Postgres, "obs"), vec!["obs"]);
        assert_eq!(label_keys(SqlDialect::Sqlite, "Obs"), vec!["obs"]);
    }
}
