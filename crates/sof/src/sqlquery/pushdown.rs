//! Predicate push-down for dependency ViewDefinitions (#2574).
//!
//! A SQL query over ViewDefinition tables materializes each dependency in
//! full before its own `WHERE` runs, so a query that keeps one patient's rows
//! out of a million pays for extracting, transferring and inserting the
//! million. This module takes the simplest and most common filters of the
//! query's `WHERE` and turns them into `where` entries of the dependency
//! ViewDefinition, so the store yields only the rows the SQL can keep. The
//! SQL still runs its whole `WHERE` over what arrives.
//!
//! Soundness is the one requirement: a pushed filter must keep every row the
//! SQL's `WHERE` would keep. Both halves are conservative about it:
//!
//! - [`pushable_predicates`] parses the SQL and takes only a `SELECT` whose
//!   `FROM` is exactly the one table, with no join, subquery, CTE or set
//!   operation, and from its `WHERE` only the top-level `AND` conjuncts of the
//!   forms `col = literal`, `col <> literal`, `col IS NULL` and
//!   `col IS NOT NULL` (either operand order; `t.col` with the table's own
//!   name or alias). Any other conjunct is left to the SQL. An `OR` at the
//!   top level pushes nothing.
//! - [`push_down_view_definition`] turns a predicate into FHIRPath only for a
//!   column declared at the top level of the view (not under `forEach`,
//!   `forEachOrNull`, `unionAll`, `repeat` or a nested `select`), with a
//!   plain navigation `path`, `collection` unset or `false`, and a declared
//!   `type` whose comparison semantics match the literal's: string literals
//!   on string-family types (`string`, `code`, `id`, `uri`, …), numeric
//!   literals of at most 15 significant digits on `integer`, `integer64`, `positiveInt`, `unsignedInt` and
//!   `decimal`, boolean literals on `boolean`. Date, time and binary types are
//!   never compared (a string literal against a FHIRPath date is not the text
//!   comparison SQL makes), and a column without a declared `type` is never
//!   compared, since the engine's affinity-based comparison can match where
//!   FHIRPath would not. `IS NULL` / `IS NOT NULL` need no type.
//!
//! The pushed `where` reads the column's own `path` (`(path) = 'x'`,
//! `(path).exists()`), so it selects exactly the resources whose column holds
//! that value, and a view with `where` entries already keeps them.

use serde_json::{Map, Value, json};
use sqlparser::ast::{
    BinaryOperator, Expr, Ident, ObjectName, Query, SetExpr, Statement, TableFactor,
    Value as SqlValue,
};
use sqlparser::dialect::SQLiteDialect;
use sqlparser::parser::Parser;

/// A literal a predicate compares a column with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Literal {
    /// A single-quoted SQL string, unescaped.
    Str(String),
    /// A numeric literal, as written (digits, an optional sign and point).
    Num(String),
    Bool(bool),
}

/// One conjunct of a SQL `WHERE` that can be pushed into a dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Predicate {
    Eq { column: String, literal: Literal },
    Ne { column: String, literal: Literal },
    IsNull { column: String },
    IsNotNull { column: String },
}

impl Predicate {
    /// The column the predicate reads, lower-cased.
    pub fn column(&self) -> &str {
        match self {
            Predicate::Eq { column, .. }
            | Predicate::Ne { column, .. }
            | Predicate::IsNull { column }
            | Predicate::IsNotNull { column } => column,
        }
    }
}

/// The predicates of `sql`'s `WHERE` that may be pushed into the table
/// `table` (a `depends-on` label), or `None` when the SQL is not one plain
/// `SELECT` over that one table, or has no `WHERE` worth pushing.
///
/// Column names are lower-cased; the SQL engine resolves them without regard
/// to case, as the ViewDefinition's column names are matched here.
pub fn pushable_predicates(sql: &str, table: &str) -> Option<Vec<Predicate>> {
    let statements = Parser::parse_sql(&SQLiteDialect {}, sql).ok()?;
    let [Statement::Query(query)] = statements.as_slice() else {
        return None;
    };
    // A subquery anywhere in the statement could read the same table under
    // another name, and would then see the pushed filter too: push nothing.
    if has_nested_query(query) {
        return None;
    }
    let select = single_table_select(query)?;
    let (relation, alias) = match select.from.as_slice() {
        [from] if from.joins.is_empty() => match &from.relation {
            TableFactor::Table {
                name, alias, args, ..
            } if args.is_none() => (name, alias.as_ref().map(|a| &a.name)),
            _ => return None,
        },
        _ => return None,
    };
    if !object_name_is(relation, table) {
        return None;
    }
    // A column may be qualified by the table's name or its alias.
    let qualifiers: Vec<String> = std::iter::once(table.to_lowercase())
        .chain(alias.map(|a| a.value.to_lowercase()))
        .collect();
    let selection = select.selection.as_ref()?;
    let mut conjuncts = Vec::new();
    collect_conjuncts(selection, &mut conjuncts);
    let predicates: Vec<Predicate> = conjuncts
        .into_iter()
        .filter_map(|expr| predicate_of(expr, &qualifiers))
        .collect();
    if predicates.is_empty() {
        None
    } else {
        Some(predicates)
    }
}

/// `query` as the one `SELECT` it is, or `None` when it has a CTE or a set
/// operation.
fn single_table_select(query: &Query) -> Option<&sqlparser::ast::Select> {
    if query.with.is_some() {
        return None;
    }
    match query.body.as_ref() {
        SetExpr::Select(select) => Some(select),
        _ => None,
    }
}

/// True when `query` contains another query anywhere: a subquery in the
/// projection, `WHERE`, `HAVING`, a derived table, an `IN (SELECT …)`.
fn has_nested_query(query: &Query) -> bool {
    use sqlparser::ast::{Visit, Visitor};
    use std::ops::ControlFlow;
    struct Counter(usize);
    impl Visitor for Counter {
        type Break = ();
        fn pre_visit_query(&mut self, _query: &Query) -> ControlFlow<()> {
            self.0 += 1;
            if self.0 > 1 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        }
    }
    query.visit(&mut Counter(0)).is_break()
}

fn object_name_is(name: &ObjectName, table: &str) -> bool {
    matches!(name.0.as_slice(), [ident] if ident.value.eq_ignore_ascii_case(table))
}

/// The top-level `AND` conjuncts of `expr`, parentheses removed.
fn collect_conjuncts<'a>(expr: &'a Expr, out: &mut Vec<&'a Expr>) {
    match expr {
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            collect_conjuncts(left, out);
            collect_conjuncts(right, out);
        }
        Expr::Nested(inner) => collect_conjuncts(inner, out),
        other => out.push(other),
    }
}

/// `expr` as a pushable predicate, when it is one.
fn predicate_of(expr: &Expr, qualifiers: &[String]) -> Option<Predicate> {
    match expr {
        Expr::Nested(inner) => predicate_of(inner, qualifiers),
        Expr::IsNull(inner) => Some(Predicate::IsNull {
            column: column_of(inner, qualifiers)?,
        }),
        Expr::IsNotNull(inner) => Some(Predicate::IsNotNull {
            column: column_of(inner, qualifiers)?,
        }),
        Expr::BinaryOp { left, op, right } => {
            let (column, literal) = match (column_of(left, qualifiers), literal_of(right)) {
                (Some(column), Some(literal)) => (column, literal),
                _ => (column_of(right, qualifiers)?, literal_of(left)?),
            };
            match op {
                BinaryOperator::Eq => Some(Predicate::Eq { column, literal }),
                BinaryOperator::NotEq => Some(Predicate::Ne { column, literal }),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The column `expr` names, lower-cased: a bare identifier, or one qualified
/// by the table's name or alias.
fn column_of(expr: &Expr, qualifiers: &[String]) -> Option<String> {
    match expr {
        Expr::Identifier(ident) => Some(ident_value(ident)),
        Expr::CompoundIdentifier(parts) => match parts.as_slice() {
            [qualifier, column] if qualifiers.contains(&ident_value(qualifier)) => {
                Some(ident_value(column))
            }
            _ => None,
        },
        Expr::Nested(inner) => column_of(inner, qualifiers),
        _ => None,
    }
}

fn ident_value(ident: &Ident) -> String {
    ident.value.to_lowercase()
}

fn literal_of(expr: &Expr) -> Option<Literal> {
    match expr {
        Expr::Value(SqlValue::SingleQuotedString(text)) => Some(Literal::Str(text.clone())),
        Expr::Value(SqlValue::Boolean(value)) => Some(Literal::Bool(*value)),
        Expr::Value(SqlValue::Number(text, _))
            if is_plain_number(text) && significant_digits(text) <= MAX_PUSHED_DIGITS =>
        {
            Some(Literal::Num(text.clone()))
        }
        Expr::Nested(inner) => literal_of(inner),
        _ => None,
    }
}

/// The most significant digits a pushed numeric literal may have.
///
/// Two reasons, both about keeping every row the SQL would keep. A FHIRPath
/// integer is an `i64` and a decimal a `rust_decimal`, so a longer literal
/// (`99999999999999999999`) does not parse and the view fails instead of
/// returning the rows. And the SQL engine compares numbers as `f64`, while a
/// store may compare the pushed `where` exactly (PostgreSQL's `numeric`), so
/// `= 5.0000000000000001` is `5` to the SQL but not to the store. Any decimal
/// of at most 15 significant digits survives the round trip through `f64`.
const MAX_PUSHED_DIGITS: usize = 15;

/// The significant digits of a plain number: its digits without the point,
/// leading zeros and a fraction's trailing zeros.
fn significant_digits(text: &str) -> usize {
    let text = if text.contains('.') {
        text.trim_end_matches('0')
    } else {
        text
    };
    text.chars()
        .filter(char::is_ascii_digit)
        .skip_while(|c| *c == '0')
        .count()
}

/// Digits with at most one point: the shape of a FHIRPath number literal.
/// Exponents and other spellings are not pushed.
fn is_plain_number(text: &str) -> bool {
    let mut points = 0;
    !text.is_empty()
        && text.chars().all(|c| {
            if c == '.' {
                points += 1;
                true
            } else {
                c.is_ascii_digit()
            }
        })
        && points <= 1
        && !text.starts_with('.')
        && !text.ends_with('.')
}

/// The type family a declared column `type` belongs to, for matching a
/// literal; `None` for types this module never compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypeFamily {
    Str,
    Num,
    Bool,
}

fn type_family(type_code: &str) -> Option<TypeFamily> {
    match type_code {
        "boolean" => Some(TypeFamily::Bool),
        "integer" | "integer64" | "positiveInt" | "unsignedInt" | "decimal" => {
            Some(TypeFamily::Num)
        }
        "string" | "code" | "id" | "uri" | "url" | "canonical" | "oid" | "uuid" | "markdown" => {
            Some(TypeFamily::Str)
        }
        _ => None,
    }
}

fn literal_family(literal: &Literal) -> TypeFamily {
    match literal {
        Literal::Str(_) => TypeFamily::Str,
        Literal::Num(_) => TypeFamily::Num,
        Literal::Bool(_) => TypeFamily::Bool,
    }
}

/// A column the predicates may apply to: a top-level column of `view` with a
/// plain navigation path and no collection, keyed by its lower-cased name.
struct TopLevelColumn {
    path: String,
    type_code: Option<String>,
}

fn top_level_columns(view: &Value) -> Vec<(String, TopLevelColumn)> {
    let mut out = Vec::new();
    let Some(selects) = view.get("select").and_then(Value::as_array) else {
        return out;
    };
    for select in selects {
        let Some(select) = select.as_object() else {
            continue;
        };
        // Only a select that is nothing but columns: its paths read the
        // resource itself, as a `where` does.
        if select.keys().any(|key| {
            !matches!(
                key.as_str(),
                "column" | "id" | "extension" | "modifierExtension"
            )
        }) {
            continue;
        }
        let Some(columns) = select.get("column").and_then(Value::as_array) else {
            continue;
        };
        for column in columns {
            let (Some(name), Some(path)) = (
                column.get("name").and_then(Value::as_str),
                column.get("path").and_then(Value::as_str),
            ) else {
                continue;
            };
            if column.get("collection").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            if !is_plain_navigation(path) {
                continue;
            }
            out.push((
                name.to_lowercase(),
                TopLevelColumn {
                    path: path.to_string(),
                    type_code: column
                        .get("type")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                },
            ));
        }
    }
    out
}

/// A path of identifiers joined by `.`, with `first()` and `last()` steps
/// allowed: the shape whose value the column holds verbatim, so comparing
/// it in a `where` compares the column.
fn is_plain_navigation(path: &str) -> bool {
    !path.is_empty()
        && path.split('.').all(|step| {
            step == "first()"
                || step == "last()"
                || (!step.is_empty()
                    && step
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '`'))
        })
}

/// FHIRPath string literal of `text`.
fn fhirpath_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('\'');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('\'');
    out
}

fn fhirpath_literal(literal: &Literal) -> String {
    match literal {
        Literal::Str(text) => fhirpath_string(text),
        Literal::Num(text) => text.clone(),
        Literal::Bool(value) => value.to_string(),
    }
}

/// `view` with a `where` entry for every predicate that applies to one of its
/// top-level typed columns, or `None` when none applies. Predicates that do
/// not apply are simply not pushed; the SQL still enforces them.
pub fn push_down_view_definition(view: &Value, predicates: &[Predicate]) -> Option<Value> {
    let columns = top_level_columns(view);
    let mut pushed: Vec<Value> = Vec::new();
    for predicate in predicates {
        let Some((_, column)) = columns.iter().find(|(name, _)| name == predicate.column()) else {
            continue;
        };
        let expr = match predicate {
            Predicate::IsNull { .. } => format!("({}).empty()", column.path),
            Predicate::IsNotNull { .. } => format!("({}).exists()", column.path),
            Predicate::Eq { literal, .. } | Predicate::Ne { literal, .. } => {
                let family = column.type_code.as_deref().and_then(type_family);
                if family != Some(literal_family(literal)) {
                    continue;
                }
                let op = if matches!(predicate, Predicate::Eq { .. }) {
                    "="
                } else {
                    "!="
                };
                format!("({}) {op} {}", column.path, fhirpath_literal(literal))
            }
        };
        pushed.push(json!({ "path": expr }));
    }
    if pushed.is_empty() {
        return None;
    }
    let mut out = view.clone();
    let object: &mut Map<String, Value> = out.as_object_mut()?;
    let wheres = object
        .entry("where")
        .or_insert_with(|| Value::Array(Vec::new()));
    match wheres.as_array_mut() {
        Some(existing) => existing.extend(pushed),
        None => return None,
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eq(column: &str, literal: Literal) -> Predicate {
        Predicate::Eq {
            column: column.into(),
            literal,
        }
    }

    #[test]
    fn takes_top_level_conjuncts_in_either_operand_order_and_with_a_qualifier() {
        let predicates = pushable_predicates(
            "SELECT id FROM obs o WHERE o.status = 'final' AND 5 = value AND (o.unit IS NOT NULL) \
             AND flag = true AND lower(code) = 'x' AND note IS NULL",
            "obs",
        )
        .expect("a plain select over obs");
        assert_eq!(
            predicates,
            vec![
                eq("status", Literal::Str("final".into())),
                eq("value", Literal::Num("5".into())),
                Predicate::IsNotNull {
                    column: "unit".into()
                },
                eq("flag", Literal::Bool(true)),
                Predicate::IsNull {
                    column: "note".into()
                },
            ]
        );
    }

    /// A numeric literal past 15 significant digits is left to the SQL: past
    /// `i64` it does not parse as FHIRPath (the view failed with a 500), and
    /// past `f64` precision the store's exact comparison drops rows the SQL's
    /// float comparison keeps (`= 5.0000000000000001` on PostgreSQL).
    #[test]
    fn a_numeric_literal_past_fifteen_significant_digits_is_not_pushed() {
        let predicates = pushable_predicates(
            "SELECT id FROM t WHERE a = 99999999999999999999 AND b = 9223372036854775808 \
             AND c = 5.0000000000000001 AND d = 0.1000000000000000000001 \
             AND e = 123456789012345 AND f = 0.000000000000000000001 AND g = 1.500000000000000000 \
             AND h = 00000000000000000000042",
            "t",
        )
        .expect("the short literals are pushable");
        assert_eq!(
            predicates,
            vec![
                eq("e", Literal::Num("123456789012345".into())),
                eq("f", Literal::Num("0.000000000000000000001".into())),
                eq("g", Literal::Num("1.500000000000000000".into())),
                eq("h", Literal::Num("00000000000000000000042".into())),
            ]
        );

        let columns: Vec<Value> = ["e", "f", "g", "h"]
            .map(|name| json!({"name": name, "path": "valueQuantity.value", "type": "decimal"}))
            .into();
        let view = json!({
            "resourceType": "ViewDefinition", "resource": "Observation",
            "select": [{"column": columns}]
        });
        let pushed = push_down_view_definition(&view, &predicates).expect("pushed");
        for entry in pushed["where"].as_array().expect("where entries") {
            let path = entry["path"].as_str().expect("a path");
            helios_fhirpath::parse_expression(path)
                .unwrap_or_else(|e| panic!("pushed `{path}` must parse as FHIRPath: {e}"));
        }
    }

    #[test]
    fn significant_digits_ignore_the_point_leading_zeros_and_fraction_trailing_zeros() {
        assert_eq!(significant_digits("0"), 0);
        assert_eq!(significant_digits("100"), 3);
        assert_eq!(significant_digits("007"), 1);
        assert_eq!(significant_digits("0.0010"), 1);
        assert_eq!(significant_digits("12.340"), 4);
        assert_eq!(significant_digits("5.0000000000000001"), 17);
    }

    #[test]
    fn not_equal_is_pushed_and_other_comparisons_are_not() {
        let predicates =
            pushable_predicates("select * from t where a <> 'x' and b != 2 and c > 3", "t")
                .expect("pushable");
        assert_eq!(
            predicates,
            vec![
                Predicate::Ne {
                    column: "a".into(),
                    literal: Literal::Str("x".into())
                },
                Predicate::Ne {
                    column: "b".into(),
                    literal: Literal::Num("2".into())
                },
            ]
        );
    }

    #[test]
    fn an_or_at_the_top_level_joins_subqueries_ctes_and_other_tables_push_nothing() {
        for sql in [
            "SELECT id FROM obs WHERE status = 'final' OR status = 'amended'",
            "SELECT id FROM obs JOIN pat ON pat.id = obs.subject WHERE obs.status = 'final'",
            "SELECT id FROM obs WHERE status = 'final' AND id IN (SELECT id FROM other)",
            "WITH x AS (SELECT 1) SELECT id FROM obs WHERE status = 'final'",
            "SELECT id FROM obs WHERE status = 'final' UNION SELECT id FROM obs",
            "SELECT id FROM other WHERE status = 'final'",
            "SELECT (SELECT count(*) FROM obs) FROM obs WHERE status = 'final'",
            "SELECT id FROM obs",
            "SELECT id FROM obs WHERE other.status = 'final'",
            "SELECT id FROM obs WHERE status = 'final' AND value = -1 AND v = 1e3",
            "SELECT id FROM obs WHERE value = -1",
        ] {
            let predicates = pushable_predicates(sql, "obs");
            assert!(
                predicates.is_none() || sql.contains("-1 AND"),
                "{sql}: {predicates:?}"
            );
        }
        // Only the pushable conjunct of the mixed one.
        assert_eq!(
            pushable_predicates(
                "SELECT id FROM obs WHERE status = 'final' AND value = -1 AND v = 1e3",
                "obs"
            ),
            Some(vec![eq("status", Literal::Str("final".into()))])
        );
    }

    fn view() -> Value {
        json!({
            "resourceType": "ViewDefinition", "resource": "Observation",
            "where": [{"path": "status.exists()"}],
            "select": [
                {"column": [
                    {"name": "id", "path": "id", "type": "id"},
                    {"name": "status", "path": "status", "type": "code"},
                    {"name": "code", "path": "code.coding.first().code", "type": "code"},
                    {"name": "value", "path": "valueQuantity.value", "type": "decimal"},
                    {"name": "count", "path": "valueInteger", "type": "integer"},
                    {"name": "flag", "path": "valueBoolean", "type": "boolean"},
                    {"name": "when", "path": "effectiveDateTime", "type": "dateTime"},
                    {"name": "untyped", "path": "note.first().text"},
                    {"name": "tags", "path": "meta.tag.code", "collection": true, "type": "code"},
                    {"name": "computed", "path": "code.coding.where(system = 'x').code", "type": "code"}
                ]},
                {"forEach": "component", "column": [
                    {"name": "comp", "path": "code.coding.first().code", "type": "code"}
                ]}
            ]
        })
    }

    fn pushed_paths(view: &Value) -> Vec<String> {
        view["where"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["path"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn typed_top_level_columns_become_where_entries_after_the_existing_ones() {
        let out = push_down_view_definition(
            &view(),
            &[
                eq("status", Literal::Str("final".into())),
                eq("code", Literal::Str("it's".into())),
                Predicate::Ne {
                    column: "value".into(),
                    literal: Literal::Num("5.5".into()),
                },
                eq("count", Literal::Num("3".into())),
                eq("flag", Literal::Bool(false)),
                Predicate::IsNull {
                    column: "untyped".into(),
                },
                Predicate::IsNotNull {
                    column: "when".into(),
                },
            ],
        )
        .expect("pushed");
        assert_eq!(
            pushed_paths(&out),
            vec![
                "status.exists()",
                "(status) = 'final'",
                "(code.coding.first().code) = 'it\\'s'",
                "(valueQuantity.value) != 5.5",
                "(valueInteger) = 3",
                "(valueBoolean) = false",
                "(note.first().text).empty()",
                "(effectiveDateTime).exists()",
            ]
        );
        // The rest of the view is untouched.
        assert_eq!(out["select"], view()["select"]);
    }

    #[test]
    fn a_literal_of_another_family_a_date_an_untyped_a_collection_a_computed_or_a_foreach_column_is_not_pushed()
     {
        let out = push_down_view_definition(
            &view(),
            &[
                eq("status", Literal::Num("1".into())),
                eq("value", Literal::Str("5".into())),
                eq("flag", Literal::Num("1".into())),
                eq("when", Literal::Str("2024-01-01".into())),
                eq("untyped", Literal::Str("x".into())),
                eq("tags", Literal::Str("x".into())),
                eq("computed", Literal::Str("x".into())),
                eq("comp", Literal::Str("x".into())),
                eq("missing", Literal::Str("x".into())),
            ],
        );
        assert!(out.is_none(), "{out:?}");
    }

    #[test]
    fn a_view_without_a_where_gets_one() {
        let mut v = view();
        v.as_object_mut().unwrap().remove("where");
        let out = push_down_view_definition(&v, &[eq("status", Literal::Str("final".into()))])
            .expect("pushed");
        assert_eq!(pushed_paths(&out), vec!["(status) = 'final'"]);
    }
}
