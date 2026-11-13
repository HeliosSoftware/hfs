//! Column projection for dependency ViewDefinitions (#2574).
//!
//! A SQL query over ViewDefinition tables rarely reads every column of every
//! view, but each view is materialized in full before the SQL runs: a
//! `count(*)` over a twenty-column view pays for extracting, transferring and
//! inserting all twenty. This module works out, from the SQL text alone, which
//! column names the SQL can possibly resolve, and drops the other columns from
//! a ViewDefinition before it is run.
//!
//! Both halves are deliberately conservative:
//!
//! - [`referenced_identifiers`] never parses the SQL. It collects every
//!   identifier-like token of the text (bare, quoted, and string literals,
//!   which SQLite may resolve as identifiers), so the result is a superset of
//!   the columns the SQL reads, wherever in the statement they are read. When
//!   the SQL depends on a table's *whole* column list (`*`, `t.*`,
//!   `NATURAL JOIN`) or cannot be tokenized, it returns `None` and nothing is
//!   pruned.
//! - [`project_view_definition`] only removes entries of `column` arrays. The
//!   `select` / `forEach` / `forEachOrNull` / `repeat` / `unionAll` / `where`
//!   structure that decides which rows the view yields is left untouched, and
//!   every `column` array keeps at least one entry, so the pruned view yields
//!   exactly the rows of the original.

use std::collections::HashSet;

use serde_json::Value;
use sqlparser::dialect::SQLiteDialect;
use sqlparser::keywords::Keyword;
use sqlparser::tokenizer::{Token, Tokenizer};

/// Every identifier-like token of `sql`, lower-cased, or `None` when the SQL
/// may depend on columns it does not name.
///
/// `None` is returned when:
///
/// - the SQL cannot be tokenized (the SQL engine reports that error itself);
/// - it selects a wildcard: a `*` that follows `SELECT`, `DISTINCT`, `ALL`,
///   a comma or a `.`. A `*` after `(` is an aggregate argument (`count(*)`)
///   and any other `*` is a multiplication; neither reads a column list;
/// - it uses `NATURAL JOIN`, which joins on every column name two tables
///   share.
///
/// Uses the same [`SQLiteDialect`] the SQL engine itself speaks.
pub fn referenced_identifiers(sql: &str) -> Option<HashSet<String>> {
    let dialect = SQLiteDialect {};
    let tokens = Tokenizer::new(&dialect, sql).tokenize().ok()?;
    let mut identifiers = HashSet::new();
    let mut previous: Option<&Token> = None;
    for token in &tokens {
        match token {
            Token::Whitespace(_) => continue,
            Token::Word(word) => {
                if word.quote_style.is_none() && word.keyword == Keyword::NATURAL {
                    return None;
                }
                identifiers.insert(word.value.to_lowercase());
            }
            // SQLite falls back to reading a string literal as an identifier
            // (and the reverse) when the literal reading does not resolve.
            Token::SingleQuotedString(text)
            | Token::DoubleQuotedString(text)
            | Token::NationalStringLiteral(text)
            | Token::EscapedStringLiteral(text)
            | Token::UnicodeStringLiteral(text) => {
                identifiers.insert(text.to_lowercase());
            }
            Token::Mul if is_wildcard_position(previous) => return None,
            _ => {}
        }
        previous = Some(token);
    }
    Some(identifiers)
}

/// True when a `*` that follows `previous` is a column wildcard, not a
/// multiplication or an aggregate's `(*)` argument.
fn is_wildcard_position(previous: Option<&Token>) -> bool {
    match previous {
        None | Some(Token::Comma | Token::Period) => true,
        Some(Token::Word(word)) => {
            word.quote_style.is_none()
                && matches!(
                    word.keyword,
                    Keyword::SELECT | Keyword::DISTINCT | Keyword::ALL
                )
        }
        _ => false,
    }
}

/// Returns `view` without the columns `referenced` cannot name, or `None`
/// when no column can be dropped.
///
/// `referenced` holds lower-cased identifiers (see [`referenced_identifiers`]);
/// column names are matched case-insensitively, as SQLite matches them. A
/// column whose name is not a plain identifier is always kept: its spelling in
/// SQL text need not match the name token for token.
///
/// A `column` array none of whose entries is referenced keeps its first one.
/// That name is then kept everywhere it is declared, so the branches of a
/// `unionAll`, which must declare the same columns, are pruned alike.
pub fn project_view_definition(view: &Value, referenced: &HashSet<String>) -> Option<Value> {
    let mut keep: HashSet<String> = HashSet::new();
    // Each pass can only add names, so this ends after at most one pass per
    // `column` array.
    loop {
        let mut added = Vec::new();
        collect_fallback_columns(view, referenced, &keep, true, &mut added);
        if added.is_empty() {
            break;
        }
        keep.extend(added);
    }
    let mut projected = view.clone();
    let mut dropped = false;
    prune_columns(&mut projected, referenced, &keep, true, &mut dropped);
    dropped.then_some(projected)
}

fn is_kept(name: &str, referenced: &HashSet<String>, keep: &HashSet<String>) -> bool {
    let plain = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !name.is_empty();
    let lower = name.to_lowercase();
    !plain || referenced.contains(&lower) || keep.contains(&lower)
}

fn column_name(column: &Value) -> Option<&str> {
    column.get("name").and_then(Value::as_str)
}

/// Child select nodes of a ViewDefinition (`root`) or of a select node.
fn child_selects(node: &Value, root: bool) -> impl Iterator<Item = &Value> {
    let keys: &[&str] = if root {
        &["select"]
    } else {
        &["select", "unionAll"]
    };
    keys.iter()
        .filter_map(move |key| node.get(*key).and_then(Value::as_array))
        .flatten()
}

/// Pushes the first column of every `column` array that would otherwise end
/// up empty.
fn collect_fallback_columns(
    node: &Value,
    referenced: &HashSet<String>,
    keep: &HashSet<String>,
    root: bool,
    added: &mut Vec<String>,
) {
    if !root
        && let Some(columns) = node.get("column").and_then(Value::as_array)
        && !columns.iter().any(|column| {
            // A nameless column is malformed; it is kept as is (see
            // `prune_columns`), so the array is not empty either way.
            column_name(column).is_none_or(|name| is_kept(name, referenced, keep))
        })
        && let Some(first) = columns.first().and_then(column_name)
    {
        added.push(first.to_lowercase());
    }
    for child in child_selects(node, root) {
        collect_fallback_columns(child, referenced, keep, false, added);
    }
}

fn prune_columns(
    node: &mut Value,
    referenced: &HashSet<String>,
    keep: &HashSet<String>,
    root: bool,
    dropped: &mut bool,
) {
    if !root && let Some(columns) = node.get_mut("column").and_then(Value::as_array_mut) {
        let before = columns.len();
        columns.retain(|column| {
            column_name(column).is_none_or(|name| is_kept(name, referenced, keep))
        });
        *dropped |= columns.len() != before;
    }
    let keys: &[&str] = if root {
        &["select"]
    } else {
        &["select", "unionAll"]
    };
    for key in keys {
        if let Some(children) = node.get_mut(*key).and_then(Value::as_array_mut) {
            for child in children {
                prune_columns(child, referenced, keep, false, dropped);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn idents(sql: &str) -> Option<Vec<String>> {
        referenced_identifiers(sql).map(|set| {
            let mut names: Vec<String> = set.into_iter().collect();
            names.sort();
            names
        })
    }

    fn referenced(names: &[&str]) -> HashSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    fn names(view: &Value) -> Vec<String> {
        fn walk(node: &Value, out: &mut Vec<String>) {
            for column in node["column"].as_array().into_iter().flatten() {
                out.push(column["name"].as_str().unwrap().to_string());
            }
            for key in ["select", "unionAll"] {
                for child in node[key].as_array().into_iter().flatten() {
                    walk(child, out);
                }
            }
        }
        let mut out = Vec::new();
        for select in view["select"].as_array().into_iter().flatten() {
            walk(select, &mut out);
        }
        out
    }

    #[test]
    fn identifiers_are_collected_from_every_clause_lower_cased() {
        let found = idents(
            "SELECT p.Gender, count(*) AS n FROM patients p JOIN obs o USING (patient_id) \
             WHERE o.\"Code\" = '8867-4' AND [status] = `final` GROUP BY 1 ORDER BY n",
        )
        .unwrap();
        for name in ["gender", "patient_id", "code", "status", "final", "n"] {
            assert!(found.contains(&name.to_string()), "{name} in {found:?}");
        }
        // String literals are kept: SQLite may read one as an identifier.
        assert!(found.contains(&"8867-4".to_string()));
    }

    #[test]
    fn count_star_and_multiplication_are_not_wildcards() {
        assert!(idents("SELECT count(*) FROM t").is_some());
        assert!(idents("SELECT COUNT( * ) FROM t").is_some());
        assert!(idents("SELECT a * 2, (b) * c, 3 * d, 'x' * e FROM t").is_some());
        assert!(idents("SELECT CASE WHEN a THEN 1 END * 2 FROM t").is_some());
    }

    #[test]
    fn wildcards_and_natural_joins_disable_projection() {
        for sql in [
            "SELECT * FROM t",
            "select distinct * from t",
            "SELECT ALL * FROM t",
            "SELECT a, * FROM t",
            "SELECT t.* FROM t",
            "SELECT count(*) FROM (SELECT * FROM t)",
            "SELECT a FROM t WHERE EXISTS (SELECT * FROM u)",
            "SELECT a FROM t NATURAL JOIN u",
            "SELECT a FROM t natural left join u",
        ] {
            assert_eq!(idents(sql), None, "{sql}");
        }
        // A column that merely is called `natural` when quoted is a name.
        assert!(idents("SELECT \"natural\" FROM t").is_some());
    }

    #[test]
    fn untokenizable_sql_disables_projection() {
        assert_eq!(idents("SELECT 'unterminated FROM t"), None);
    }

    #[test]
    fn unreferenced_columns_are_dropped_case_insensitively() {
        let view = json!({"resource":"Patient","select":[{"column":[
            {"path":"id","name":"id"},
            {"path":"gender","name":"Gender"},
            {"path":"birthDate","name":"birth_date"}]}]});
        let projected = project_view_definition(&view, &referenced(&["gender"])).unwrap();
        assert_eq!(names(&projected), ["Gender"]);
        // Everything but the columns is untouched.
        assert_eq!(projected["resource"], "Patient");
    }

    #[test]
    fn nothing_to_drop_returns_none() {
        let view = json!({"resource":"Patient","select":[{"column":[
            {"path":"id","name":"id"},{"path":"gender","name":"gender"}]}]});
        assert!(project_view_definition(&view, &referenced(&["id", "gender"])).is_none());
    }

    #[test]
    fn a_column_array_never_ends_up_empty() {
        // `count(*)`: no column is referenced, the row-producing structure
        // (the forEach and its select) must survive with one column each.
        let view = json!({"resource":"Provenance","select":[
            {"column":[{"path":"id","name":"id"},{"path":"recorded","name":"recorded"}]},
            {"forEach":"target","column":[
                {"path":"reference","name":"target_ref"},
                {"path":"display","name":"target_display"}]}]});
        let projected = project_view_definition(&view, &referenced(&[])).unwrap();
        assert_eq!(names(&projected), ["id", "target_ref"]);
        assert_eq!(projected["select"][1]["forEach"], "target");
    }

    #[test]
    fn union_branches_keep_the_same_columns() {
        // The second branch splits its columns over two selects, so its first
        // `column` array differs from the first branch's: the fallback name
        // of one must be kept in both.
        let view = json!({"resource":"Patient","select":[{"unionAll":[
            {"forEach":"telecom","column":[
                {"path":"value","name":"contact"},{"path":"system","name":"kind"}]},
            {"forEach":"address","select":[
                {"column":[{"path":"'postal'","name":"kind"}]},
                {"column":[{"path":"city","name":"contact"}]}]}]}]});
        let projected = project_view_definition(&view, &referenced(&[]));
        // contact (first branch) and kind (second branch) are both fallbacks,
        // so nothing can be dropped.
        assert!(projected.is_none());

        let view = json!({"resource":"Patient","select":[{"unionAll":[
            {"forEach":"telecom","column":[
                {"path":"value","name":"contact"},{"path":"system","name":"kind"},
                {"path":"use","name":"use"}]},
            {"forEach":"address","column":[
                {"path":"city","name":"contact"},{"path":"'postal'","name":"kind"},
                {"path":"use","name":"use"}]}]}]});
        let projected = project_view_definition(&view, &referenced(&["kind"])).unwrap();
        assert_eq!(names(&projected), ["kind", "kind"]);
    }

    #[test]
    fn columns_with_unusual_names_are_always_kept() {
        let view = json!({"resource":"Patient","select":[{"column":[
            {"path":"id","name":"id"},{"path":"gender","name":"my col"},
            {"path":"active","name":"active"}]}]});
        let projected = project_view_definition(&view, &referenced(&["id"])).unwrap();
        assert_eq!(names(&projected), ["id", "my col"]);
    }
}
