//! Pure, conservative scanner over SQL text: the `:name` placeholders a
//! query uses and the tables its `FROM`/`JOIN` clauses read, each located by
//! line, column, and Unicode-character offset/length (#841, #842).
//!
//! This module never executes or fully parses the SQL — it walks the token
//! stream [`sqlparser::tokenizer::Tokenizer::tokenize_with_location`]
//! produces and applies a handful of local, position-based rules. That
//! makes it deliberately less capable than a real parser, in exchange for
//! never mis-locating (or mis-naming) anything it *does* report: every rule
//! below is written to favor a false negative (missing a table or
//! placeholder) over a false positive.
//!
//! # What [`scan_sql`] finds
//!
//! - **Placeholders**: a [`Token::Colon`] immediately followed — no
//!   whitespace, per token-span adjacency — by an unquoted [`Token::Word`].
//!   `::` tokenizes as its own [`Token::DoubleColon`], so a cast never
//!   matches. Text inside a string literal or a comment is already one
//!   token to the tokenizer, so nothing inside either is ever inspected.
//!   Duplicate names (compared with the case SQLite keeps: exact) keep only
//!   their first occurrence.
//! - **Tables**: an identifier (bare or double-quoted) is reported when the
//!   token immediately before it is the keyword `FROM`, the keyword `JOIN`
//!   (`LEFT`/`INNER`/`CROSS`/… all end in the literal `JOIN` token this
//!   checks), or a comma inside a `FROM` list that is still open at the
//!   current parenthesis depth. An identifier immediately followed by `(`
//!   (a function call, e.g. `json_each(x)`) or by `.` (a qualified
//!   `schema.table` reference — neither part is reported) is excluded.
//!   Names declared by a `WITH [RECURSIVE] name AS (...)` clause are
//!   collected first and never reported as a table reference, wherever they
//!   are used — but any table the CTE's own body reads is still reported,
//!   since the body is scanned like any other subquery. Duplicate names
//!   (compared case-insensitively, like SQLite compares table names) keep
//!   only their first occurrence's spelling and position.
//!
//! # What it deliberately does **not** detect
//!
//! - An alias is never reported and never influences whether the table it
//!   names is reported — this module has no notion of "alias", only of
//!   "the token after a table reference wasn't `(` or `.`".
//! - A table named only inside a derived table's `(...)` — `FROM (SELECT *
//!   FROM t) sub` reports `t` (found by the ordinary `FROM` rule while
//!   scanning inside the parentheses) but never `sub`.
//! - `VALUES`, table-valued functions, and any other `FROM` item that is
//!   not a bare or quoted identifier are silently skipped, not reported as
//!   errors.
//! - A three-or-more-part qualified name (`catalog.schema.table`) is
//!   excluded the same way a two-part one is; neither segment is reported.
//! - Anything the tokenizer itself cannot lex (see [`ScanError`]) — the
//!   caller is expected to let SQL parsing report that error instead.

use std::collections::HashSet;

use sqlparser::dialect::SQLiteDialect;
use sqlparser::keywords::Keyword;
use sqlparser::tokenizer::{Location, Token, TokenWithSpan, Tokenizer};
use thiserror::Error;

/// A location inside the scanned SQL text, in two coordinate systems at
/// once: 1-based `line`/`column` (matching the `Line: N, Column: M` marker
/// [`sqlparser`]'s own parse errors already use, so a caller that extracts
/// that marker from one keeps working for the other) and a 0-based
/// Unicode-`char` `offset`/`length` pair from the start of the text — what
/// a browser text editor (CodeMirror) indexes with, since it counts code
/// points, not UTF-8 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct SourcePosition {
    /// 1-based line number.
    pub line: usize,
    /// 1-based column number.
    pub column: usize,
    /// 0-based Unicode-character offset from the start of the text.
    pub offset: usize,
    /// Length of the located span, in Unicode characters.
    pub length: usize,
}

/// A `:name` placeholder the SQL uses, located at the `:`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Placeholder {
    /// The parameter name, without the leading `:`, exactly as written —
    /// SQLite treats named-parameter case as significant.
    pub name: String,
    pub position: SourcePosition,
}

/// A table the SQL reads, located at its identifier (quotes included, when
/// quoted).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TableRef {
    /// The table name exactly as written (quotes stripped for a quoted
    /// identifier, since [`sqlparser`]'s tokenizer already does that);
    /// compare case-insensitively, as SQLite compares table names.
    pub name: String,
    pub position: SourcePosition,
}

/// The result of scanning one SQL statement: every placeholder and table
/// reference [`scan_sql`] found, each in first-occurrence order.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct ScanResult {
    pub placeholders: Vec<Placeholder>,
    pub tables: Vec<TableRef>,
}

/// A failure to tokenize the SQL text at all. Per this module's contract
/// (#841), a caller receiving this should skip linting and let the SQL
/// engine's own parser report the error instead — this scanner never tries
/// to diagnose *why* the SQL is malformed.
#[derive(Debug, Error)]
pub enum ScanError {
    #[error("SQL tokenizer error: {0}")]
    Tokenize(String),
}

/// Scans `sql` for `:name` placeholders and the tables its `FROM`/`JOIN`
/// clauses read, without executing or fully parsing it. See the module docs
/// for the exact rules and their limits.
///
/// Uses the same [`SQLiteDialect`] the SQL engine itself speaks, so a colon
/// or an identifier tokenizes identically here and at execution time.
pub fn scan_sql(sql: &str) -> Result<ScanResult, ScanError> {
    let dialect = SQLiteDialect {};
    let tokens = Tokenizer::new(&dialect, sql)
        .tokenize_with_location()
        .map_err(|e| ScanError::Tokenize(e.to_string()))?;

    // Every position callers see is computed from this text, so an offset
    // is always in `char`s, never UTF-8 bytes.
    let line_starts = compute_line_starts(sql);

    let significant: Vec<&TokenWithSpan> = tokens
        .iter()
        .filter(|t| !matches!(t.token, Token::Whitespace(_)))
        .collect();

    Ok(ScanResult {
        placeholders: scan_placeholders(&significant, &line_starts),
        tables: scan_tables(&significant, &line_starts),
    })
}

/// Returns the tables `result` found that are not present in
/// `declared_labels` (compared case-insensitively, like SQLite compares
/// table names), in the order they first appeared in the SQL.
pub fn undeclared_tables<'a>(
    result: &'a ScanResult,
    declared_labels: &[String],
) -> Vec<&'a TableRef> {
    let declared: HashSet<String> = declared_labels.iter().map(|l| l.to_lowercase()).collect();
    result
        .tables
        .iter()
        .filter(|t| !declared.contains(&t.name.to_lowercase()))
        .collect()
}

/// Offset (in `char`s) of the first character of each line, indexed by
/// `line - 1` — `line_starts[0]` is always `0`. Built by the same rule
/// [`sqlparser`]'s tokenizer uses to advance its own `line`/`column`
/// (`\n` starts a new line; every other `char` — not byte — advances the
/// column), so a [`Location`] it reports always maps back to the exact
/// offset that produced it.
fn compute_line_starts(sql: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (idx, ch) in sql.chars().enumerate() {
        if ch == '\n' {
            starts.push(idx + 1);
        }
    }
    starts
}

/// Converts a tokenizer [`Location`] (1-based line/column) to a 0-based
/// `char` offset from the start of the text, using the precomputed
/// `line_starts` table.
fn to_offset(line_starts: &[usize], loc: Location) -> usize {
    let line_idx = (loc.line as usize).saturating_sub(1);
    let line_start = line_starts
        .get(line_idx)
        .copied()
        .unwrap_or_else(|| line_starts.last().copied().unwrap_or(0));
    line_start + (loc.column as usize).saturating_sub(1)
}

fn source_position(line_starts: &[usize], start: Location, end: Location) -> SourcePosition {
    let offset = to_offset(line_starts, start);
    let end_offset = to_offset(line_starts, end);
    SourcePosition {
        line: start.line as usize,
        column: start.column as usize,
        offset,
        length: end_offset.saturating_sub(offset),
    }
}

/// Every `:name` placeholder, first occurrence only (exact-case
/// dedup — SQLite named-parameter case is significant).
fn scan_placeholders(tokens: &[&TokenWithSpan], line_starts: &[usize]) -> Vec<Placeholder> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for pair in tokens.windows(2) {
        let (colon, word) = (pair[0], pair[1]);
        if !matches!(colon.token, Token::Colon) {
            continue;
        }
        // No gap between the `:` and the identifier — a space (or, in
        // principle, a comment) in between means this isn't a named
        // parameter SQLite would recognize either.
        if colon.span.end != word.span.start {
            continue;
        }
        let Token::Word(w) = &word.token else {
            continue;
        };
        // SQLite named parameters are never quoted identifiers.
        if w.quote_style.is_some() {
            continue;
        }
        if !seen.insert(w.value.clone()) {
            continue;
        }
        out.push(Placeholder {
            name: w.value.clone(),
            position: source_position(line_starts, colon.span.start, word.span.end),
        });
    }
    out
}

/// Keywords that close an open `FROM` list at the current parenthesis
/// depth — once one of these appears, a later comma at the same depth is an
/// expression-list separator (`GROUP BY a, b`, `IN (1, 2)`, …), not another
/// table reference.
fn ends_from_list(keyword: Keyword) -> bool {
    matches!(
        keyword,
        Keyword::WHERE
            | Keyword::GROUP
            | Keyword::HAVING
            | Keyword::ORDER
            | Keyword::LIMIT
            | Keyword::UNION
            | Keyword::INTERSECT
            | Keyword::EXCEPT
            | Keyword::WINDOW
    )
}

/// Every table read via `FROM`/`JOIN`, first occurrence only
/// (case-insensitive dedup), excluding CTE names, qualified names,
/// subqueries, and function calls.
fn scan_tables(tokens: &[&TokenWithSpan], line_starts: &[usize]) -> Vec<TableRef> {
    let cte_names = collect_cte_names(tokens);

    let mut paren_depth: usize = 0;
    // One flag per open parenthesis depth: is a `FROM` list still open here?
    let mut from_active: Vec<bool> = vec![false];
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for (i, tok) in tokens.iter().enumerate() {
        match &tok.token {
            Token::LParen => {
                paren_depth += 1;
                from_active.push(false);
            }
            Token::RParen => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                    from_active.pop();
                }
            }
            Token::Word(w) if w.keyword == Keyword::FROM => {
                from_active[paren_depth] = true;
            }
            Token::Word(w) if ends_from_list(w.keyword) => {
                from_active[paren_depth] = false;
            }
            Token::Word(w) if w.keyword == Keyword::NoKeyword => {
                let is_candidate = i > 0
                    && match &tokens[i - 1].token {
                        Token::Word(pw) => {
                            pw.keyword == Keyword::FROM || pw.keyword == Keyword::JOIN
                        }
                        Token::Comma => from_active[paren_depth],
                        _ => false,
                    };
                if !is_candidate {
                    continue;
                }
                // A function call (`json_each(x)`) or a qualified name
                // (`schema.table`, `catalog.schema.table`) — neither part
                // of a qualified name is reported.
                let excluded = matches!(
                    tokens.get(i + 1).map(|t| &t.token),
                    Some(Token::LParen) | Some(Token::Period)
                );
                if excluded {
                    continue;
                }
                if cte_names.contains(&w.value.to_lowercase()) {
                    continue;
                }
                if !seen.insert(w.value.to_lowercase()) {
                    continue;
                }
                out.push(TableRef {
                    name: w.value.clone(),
                    position: source_position(line_starts, tok.span.start, tok.span.end),
                });
            }
            _ => {}
        }
    }
    out
}

/// Collects every name a `WITH [RECURSIVE] name [(cols)] AS (...)` clause
/// declares, lower-cased, so [`scan_tables`] can exclude a later `FROM
/// name`/`JOIN name` reference to it — the CTE's own body is scanned like
/// any other parenthesized subquery, so tables *it* reads are still found.
///
/// Independent of [`scan_tables`]'s own state machine on purpose: this is a
/// flat walk that only tracks parenthesis depth, so it can't mis-fire on
/// the same edge cases (nested subqueries, multiple CTEs) the table scan
/// has to reason about `FROM`-list activity for.
fn collect_cte_names(tokens: &[&TokenWithSpan]) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut i = 0usize;
    while i < tokens.len() {
        let is_with = matches!(&tokens[i].token, Token::Word(w) if w.keyword == Keyword::WITH);
        if !is_with {
            i += 1;
            continue;
        }
        i += 1;
        if matches!(tokens.get(i).map(|t| &t.token), Some(Token::Word(w)) if w.keyword == Keyword::RECURSIVE)
        {
            i += 1;
        }
        while let Some(Token::Word(name)) = tokens.get(i).map(|t| &t.token) {
            names.insert(name.value.to_lowercase());
            i += 1;
            // Optional explicit column list: `cte(col1, col2) AS (...)`.
            if matches!(tokens.get(i).map(|t| &t.token), Some(Token::LParen)) {
                i = skip_balanced_parens(tokens, i);
            }
            let is_as = matches!(tokens.get(i).map(|t| &t.token), Some(Token::Word(w)) if w.keyword == Keyword::AS);
            if !is_as {
                break;
            }
            i += 1;
            if !matches!(tokens.get(i).map(|t| &t.token), Some(Token::LParen)) {
                break;
            }
            i = skip_balanced_parens(tokens, i);
            if matches!(tokens.get(i).map(|t| &t.token), Some(Token::Comma)) {
                i += 1;
                continue; // another `name AS (...)` follows
            }
            break;
        }
    }
    names
}

/// Given the index of a `(`, returns the index just past its matching `)`.
/// If the parentheses are unbalanced (malformed SQL that still tokenized),
/// returns `tokens.len()` — the caller's loop simply ends, which is the
/// conservative choice for a scanner that never reports on input it isn't
/// sure it understood.
fn skip_balanced_parens(tokens: &[&TokenWithSpan], open_paren_idx: usize) -> usize {
    let mut depth = 0i32;
    let mut i = open_paren_idx;
    loop {
        match tokens.get(i).map(|t| &t.token) {
            Some(Token::LParen) => depth += 1,
            Some(Token::RParen) => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            Some(_) => {}
            None => return tokens.len(),
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(result: &ScanResult) -> Vec<&str> {
        result.tables.iter().map(|t| t.name.as_str()).collect()
    }

    fn placeholder_names(result: &ScanResult) -> Vec<&str> {
        result
            .placeholders
            .iter()
            .map(|p| p.name.as_str())
            .collect()
    }

    #[test]
    fn simple_from_reports_table_and_position() {
        let result = scan_sql("SELECT * FROM v").unwrap();
        assert_eq!(names(&result), vec!["v"]);
        assert_eq!(result.tables[0].position.line, 1);
        assert_eq!(result.tables[0].position.column, 15);
        assert_eq!(result.tables[0].position.offset, 14);
        assert_eq!(result.tables[0].position.length, 1);
    }

    #[test]
    fn join_variants_report_both_sides() {
        let result = scan_sql("SELECT * FROM a LEFT JOIN b ON a.id = b.id").unwrap();
        assert_eq!(names(&result), vec!["a", "b"]);
    }

    #[test]
    fn comma_list_reports_every_item_ignoring_aliases() {
        let result = scan_sql("SELECT * FROM a, b c, d AS e").unwrap();
        assert_eq!(names(&result), vec!["a", "b", "d"]);
    }

    #[test]
    fn cte_body_reported_but_cte_reference_excluded() {
        let result = scan_sql("WITH cte AS (SELECT * FROM t) SELECT * FROM cte").unwrap();
        assert_eq!(names(&result), vec!["t"]);
    }

    #[test]
    fn subquery_in_from_reports_nothing() {
        let result = scan_sql("SELECT * FROM (SELECT 1) sub").unwrap();
        assert!(result.tables.is_empty());
    }

    #[test]
    fn table_valued_function_reports_nothing() {
        let result = scan_sql("SELECT * FROM json_each(x)").unwrap();
        assert!(result.tables.is_empty());
    }

    #[test]
    fn quoted_identifier_reports_unquoted_name() {
        let result = scan_sql(r#"SELECT * FROM "Quoted""#).unwrap();
        assert_eq!(names(&result), vec!["Quoted"]);
    }

    #[test]
    fn qualified_name_reports_nothing() {
        let result = scan_sql("SELECT * FROM main.t").unwrap();
        assert!(result.tables.is_empty());
    }

    #[test]
    fn dedup_is_case_insensitive_and_keeps_first_spelling() {
        let result = scan_sql("SELECT * FROM v v2 JOIN V").unwrap();
        assert_eq!(names(&result), vec!["v"]);
    }

    #[test]
    fn placeholder_ignores_literal_and_comment_reports_real_one() {
        let result = scan_sql("SELECT * WHERE x = ':notaparam' AND y = :ward -- :comment").unwrap();
        assert_eq!(placeholder_names(&result), vec!["ward"]);
    }

    #[test]
    fn double_colon_cast_is_not_a_placeholder() {
        let result = scan_sql("SELECT x::int").unwrap();
        assert!(result.placeholders.is_empty());
    }

    #[test]
    fn placeholder_dedup_keeps_first_occurrence() {
        let result = scan_sql("SELECT * WHERE a = :p AND b = :p").unwrap();
        assert_eq!(result.placeholders.len(), 1);
    }

    #[test]
    fn multibyte_text_offsets_count_chars_not_bytes() {
        // "café" is 4 chars / 5 bytes; the placeholder starts right after it.
        let sql = "SELECT * WHERE name = 'café' AND x = :p";
        let result = scan_sql(sql).unwrap();
        assert_eq!(placeholder_names(&result), vec!["p"]);
        let pos = result.placeholders[0].position;
        assert_eq!(pos.offset, sql.chars().count() - 2);
    }

    #[test]
    fn multiline_sql_reports_correct_line_and_column() {
        let sql = "SELECT *\nFROM v\nWHERE x = :p";
        let result = scan_sql(sql).unwrap();
        assert_eq!(result.tables[0].position.line, 2);
        assert_eq!(result.tables[0].position.column, 6);
        assert_eq!(result.placeholders[0].position.line, 3);
        assert_eq!(result.placeholders[0].position.column, 11);
    }

    #[test]
    fn tokenizer_failure_is_reported_as_scan_error() {
        // An unterminated string literal cannot be tokenized at all.
        let err = scan_sql("SELECT 'unterminated").unwrap_err();
        assert!(matches!(err, ScanError::Tokenize(_)));
    }

    #[test]
    fn undeclared_tables_helper_is_case_insensitive_and_ordered() {
        let result = scan_sql("SELECT * FROM a JOIN B JOIN c").unwrap();
        let declared = vec!["A".to_string(), "c".to_string()];
        let undeclared = undeclared_tables(&result, &declared);
        assert_eq!(
            undeclared
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>(),
            vec!["B"]
        );
    }
}
