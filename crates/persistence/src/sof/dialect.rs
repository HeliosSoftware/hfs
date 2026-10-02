//! Dialect trait — token-level SQL emission for PostgreSQL JSONB and SQLite JSON1.
//!
//! The compiler builds dialect-independent IR ([`PlanNode`](super::ir::PlanNode)
//! and [`SqlExpr`](super::ir::SqlExpr)); the emitter walks the IR and asks the
//! dialect for each concrete SQL token. Keeping these helpers behind a trait
//! confines per-dialect divergence (operator syntax, parameter form, JSON
//! function names) to two small implementations.

#![allow(dead_code)] // Stage 1 scaffold; consumers land in stages 2–5.

use super::ir::{JsonType, SqlType};

// ============================================================================
// Literal safety (member names and string values)
// ============================================================================
//
// Caller-influenced strings reach the SQL text only as string literals, and
// every one of them is rendered by the helpers below (or by the
// [`Dialect::string_literal`] method that wraps them), never by wrapping
// quotes around the text by hand: a FHIRPath string literal, a `join()`
// separator, a `getReferenceKey(T)` `LIKE` pattern, and the member names
// described next. Run-time values (constants, runner filters) are not inlined
// at all; they are bound as `$N` / `?N` parameters.
//
// FHIRPath member names reach the SQL text inside string literals: the key in
// `base->'key'`, the elements of a `#>'{a,b}'` path array, and the segments of
// a SQLite JSON path (`'$.a.b'`). The FHIRPath parser accepts backtick-delimited
// identifiers containing *any* character, so a member name is not guaranteed to
// be a plain identifier and must never be spliced into a literal verbatim.
//
// Two layers guard against this:
//
// 1. The compiler (`compile_path`) rejects non-plain member names up front
//    with a clear error (see [`is_plain_member_name`]).
// 2. Every place that embeds a name in SQL goes through the helpers below,
//    which quote and escape unconditionally. That is the backstop: the
//    [`Dialect`] methods are infallible, so they cannot reject a name, but
//    they can guarantee that no name, plain or not, terminates the literal
//    it is embedded in.

/// True when `name` is a plain FHIRPath identifier: `[A-Za-z_][A-Za-z0-9_]*`.
///
/// Every FHIR element name matches. Names outside this set can only be written
/// as backtick-delimited identifiers, and the in-DB runner refuses to navigate
/// them rather than guess how each backend would address such a key.
pub(super) fn is_plain_member_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// SQLite single-quoted string literal. Quotes are doubled; SQLite treats
/// backslash literally inside `'...'`. NUL cannot appear in SQL text and is
/// dropped here as a last resort: callers that inline a caller-supplied
/// *value* reject NUL first (see `emit::lower_string_literal`) so the value is
/// never silently altered.
pub(super) fn sqlite_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        match c {
            '\0' => {}
            '\'' => out.push_str("''"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// PostgreSQL string literal. Quotes are doubled. Backslash is only literal
/// when `standard_conforming_strings` is on, so any string containing one is
/// emitted as an `E'...'` escape string with the backslash doubled, which is
/// correct under either setting. NUL is dropped (PG text cannot hold it); as
/// with [`sqlite_string_literal`], callers inlining a caller-supplied value
/// reject NUL first.
pub(super) fn pg_string_literal(s: &str) -> String {
    let escape_form = s.contains('\\');
    let mut out = String::with_capacity(s.len() + 3);
    if escape_form {
        out.push('E');
    }
    out.push('\'');
    for c in s.chars() {
        match c {
            '\0' => {}
            '\'' => out.push_str("''"),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// Appends one object-member step to a SQLite JSON path being built in `path`.
/// Plain identifiers become `.name`; anything else becomes a quoted label
/// (`."a b"`) with `\` and `"` backslash-escaped.
fn push_sqlite_member(path: &mut String, name: &str) {
    path.push('.');
    if is_plain_member_name(name) {
        path.push_str(name);
    } else {
        path.push('"');
        for c in name.chars() {
            if c == '\\' || c == '"' {
                path.push('\\');
            }
            path.push(c);
        }
        path.push('"');
    }
}

/// Complete SQL literal (quotes included) holding the SQLite JSON path for
/// `segments`, e.g. `'$.name[0].family'`. All-digit segments are array indices
/// (`[N]`); every other segment is an object member.
pub(super) fn sqlite_json_path_literal(segments: &[&str]) -> String {
    let mut path = String::from("$");
    for seg in segments {
        if !seg.is_empty() && seg.chars().all(|c| c.is_ascii_digit()) {
            path.push('[');
            path.push_str(seg);
            path.push(']');
        } else {
            push_sqlite_member(&mut path, seg);
        }
    }
    sqlite_string_literal(&path)
}

/// Complete SQL literal (quotes included) for the key operand of PostgreSQL's
/// `->` / `->>`, e.g. `'family'`.
pub(super) fn pg_key_literal(key: &str) -> String {
    pg_string_literal(key)
}

/// Complete SQL literal (quotes included) holding the `text[]` path operand of
/// PostgreSQL's `#>` / `#>>`, e.g. `'{name,0,family}'`.
///
/// Elements made only of letters, digits and `_` stay bare; any other element
/// (including the empty string and `null`) is double-quoted per the
/// array-literal grammar (with `\` and `"` escaped), so `,`, `{`, `}`,
/// whitespace and quotes in a name cannot split or close the array. The
/// result is then quoted as a SQL string.
pub(super) fn pg_path_array_literal(segments: &[&str]) -> String {
    let elems: Vec<String> = segments
        .iter()
        .map(|seg| {
            // A bare `NULL` element would be read as SQL NULL, so it is quoted
            // like any other special case.
            if !seg.is_empty()
                && !seg.eq_ignore_ascii_case("null")
                && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                (*seg).to_string()
            } else {
                let mut e = String::with_capacity(seg.len() + 2);
                e.push('"');
                for c in seg.chars() {
                    if c == '\\' || c == '"' {
                        e.push('\\');
                    }
                    e.push(c);
                }
                e.push('"');
                e
            }
        })
        .collect();
    pg_string_literal(&format!("{{{}}}", elems.join(",")))
}

/// Per-dialect SQL emission helpers.
pub trait Dialect: Send + Sync {
    /// Short identifier for diagnostics ("postgres", "sqlite").
    fn name(&self) -> &'static str;

    /// Render a 1-based parameter placeholder (`$1` for PG, `?1` for SQLite).
    fn placeholder(&self, idx: usize) -> String;

    /// Render `s` as a complete SQL string literal, quotes included.
    ///
    /// Every string that is inlined into SQL text (FHIRPath string literals,
    /// `join()` separators, `LIKE` patterns, ...) must go through this method
    /// rather than being wrapped in quotes by hand. SQLite doubles quotes;
    /// PostgreSQL does too, and switches to an `E'...'` escape string when `s`
    /// contains a backslash, so the literal means the same thing whether
    /// `standard_conforming_strings` is on or off. NUL cannot be represented
    /// and is dropped; callers reject it beforehand when `s` is a value.
    fn string_literal(&self, s: &str) -> String;

    /// `base->'key'` (returns JSON value).
    ///
    /// `key` is quoted and escaped for the dialect, so any string is safe to
    /// pass; the same holds for the segments of the path accessors below.
    fn json_field(&self, base: &str, key: &str) -> String;

    /// `base->>'key'` (returns text).
    fn json_field_text(&self, base: &str, key: &str) -> String;

    /// Multi-key path returning a JSON value. All-digit segments address array
    /// elements.
    fn json_path(&self, base: &str, segments: &[&str]) -> String;

    /// Multi-key path returning text.
    fn json_path_text(&self, base: &str, segments: &[&str]) -> String;

    /// Emit a lateral unnest source clause (e.g. `jsonb_array_elements(<expr>)`
    /// or `json_each(<expr>)`).
    fn unnest_array(&self, expr: &str) -> String;

    /// Emit `<expr> IS NULL`-safe wrapping for an array source — guards against
    /// `jsonb_array_elements(NULL)` / `json_each(NULL)` errors. Returns SQL that
    /// always yields a usable array (empty if missing).
    fn coalesce_array(&self, expr: &str) -> String;

    /// JSON type-of expression (`jsonb_typeof(x)` / `json_type(x)`), returning
    /// a lowercase string.
    fn json_type(&self, expr: &str) -> String;

    /// JSON aggregate (`jsonb_agg(x)` / `json_group_array(x)`).
    fn json_agg(&self, expr: &str) -> String;

    /// String aggregate with separator (`string_agg` / `group_concat`).
    fn string_agg(&self, expr: &str, sep_param: &str) -> String;

    /// SQL boolean literal for `true`.
    fn bool_true(&self) -> &'static str;
    /// SQL boolean literal for `false`.
    fn bool_false(&self) -> &'static str;

    /// `LATERAL` keyword (PG) or empty (SQLite — uses correlated subqueries).
    fn lateral_keyword(&self) -> &'static str;

    /// Cast `inner` to `ty`, returning a SQL expression.
    fn cast(&self, inner: &str, ty: SqlType) -> String;

    /// Predicate testing whether `expr` has the given JSON type.
    fn has_json_type(&self, expr: &str, ty: JsonType) -> String;

    /// Boolean coercion at the WHERE-clause boundary — represents FHIRPath's
    /// three-valued-logic rule that an empty / NULL operand filters the row
    /// out. The expression `expr` may be a text projection (PG `->>`), a JSON
    /// value (PG `->`), or a SQLite JSON1 extracted scalar; the dialect picks
    /// an appropriate form.
    fn truthy_predicate(&self, expr: &str) -> String;

    /// Substring of `s` after the last `/` — used by `getReferenceKey()` to
    /// extract the id portion of a FHIR `Reference.reference` like
    /// `Patient/123` (or `http://server/path/Patient/123`).
    fn last_path_segment(&self, s: &str) -> String;
}

// ============================================================================
// PostgreSQL
// ============================================================================

/// PostgreSQL JSONB dialect.
#[derive(Debug, Default, Clone, Copy)]
pub struct PgDialect;

impl Dialect for PgDialect {
    fn name(&self) -> &'static str {
        "postgres"
    }

    fn placeholder(&self, idx: usize) -> String {
        format!("${idx}")
    }

    fn string_literal(&self, s: &str) -> String {
        pg_string_literal(s)
    }

    fn json_field(&self, base: &str, key: &str) -> String {
        format!("{base}->{}", pg_key_literal(key))
    }

    fn json_field_text(&self, base: &str, key: &str) -> String {
        format!("{base}->>{}", pg_key_literal(key))
    }

    fn json_path(&self, base: &str, segments: &[&str]) -> String {
        if segments.len() == 1 {
            self.json_field(base, segments[0])
        } else {
            format!("{base}#>{}", pg_path_array_literal(segments))
        }
    }

    fn json_path_text(&self, base: &str, segments: &[&str]) -> String {
        if segments.len() == 1 {
            self.json_field_text(base, segments[0])
        } else {
            format!("{base}#>>{}", pg_path_array_literal(segments))
        }
    }

    fn unnest_array(&self, expr: &str) -> String {
        format!("jsonb_array_elements({expr})")
    }

    fn coalesce_array(&self, expr: &str) -> String {
        format!("coalesce({expr}, '[]'::jsonb)")
    }

    fn json_type(&self, expr: &str) -> String {
        format!("jsonb_typeof({expr})")
    }

    fn json_agg(&self, expr: &str) -> String {
        // PG's `jsonb_agg` returns NULL for empty input; coalesce to `[]`
        // so `collection: true` columns always project an array (matching
        // SQLite's `json_group_array`, which already returns `[]` for the
        // empty case).
        format!("coalesce(jsonb_agg({expr}), '[]'::jsonb)")
    }

    fn string_agg(&self, expr: &str, sep_param: &str) -> String {
        format!("string_agg({expr}, {sep_param})")
    }

    fn bool_true(&self) -> &'static str {
        "true"
    }

    fn bool_false(&self) -> &'static str {
        "false"
    }

    fn lateral_keyword(&self) -> &'static str {
        "LATERAL "
    }

    fn cast(&self, inner: &str, ty: SqlType) -> String {
        match ty {
            SqlType::Text => format!("({inner})::text"),
            // Numeric column projections wrap with an outer `::text` so the
            // PG row mapper (which reads each column as `Option<String>` to
            // stay type-agnostic) can decode the value. Round-tripping
            // through numeric first preserves canonical formatting (`1.0`
            // stays `1.0`, not `1`); the runner then JSON-parses the text
            // back to a number.
            SqlType::Integer => format!("(({inner})::bigint)::text"),
            SqlType::Decimal => format!("(({inner})::numeric)::text"),
            // Column projections want JSON-parsable text: literal `'true'` /
            // `'false'` deserialise as JSON booleans in the row mapper. The
            // input may be either a JSON `->>` text projection (`'true'` /
            // `'false'` / NULL) or a native boolean expression (e.g. a
            // comparison `(a = b)` projected through `type: boolean`); both
            // shapes cast cleanly via `::boolean` and route through `IS
            // TRUE` / `IS FALSE` to give the right text literal back.
            SqlType::Boolean => {
                format!(
                    "CASE WHEN ({inner})::boolean IS TRUE THEN 'true' \
                     WHEN ({inner})::boolean IS FALSE THEN 'false' END"
                )
            }
            SqlType::Json => format!("({inner})::jsonb"),
        }
    }

    fn has_json_type(&self, expr: &str, ty: JsonType) -> String {
        let name = match ty {
            JsonType::Object => "object",
            JsonType::Array => "array",
            JsonType::String => "string",
            JsonType::Number => "number",
            JsonType::Boolean => "boolean",
            JsonType::Null => "null",
        };
        format!("jsonb_typeof({expr}) = '{name}'")
    }

    fn truthy_predicate(&self, expr: &str) -> String {
        // Already-boolean SQL fragments (e.g. `x IS NOT NULL`) cast back to
        // boolean cheaply; text JSON projections (`r.data->>'active'`)
        // require an explicit `::boolean` cast since `IS TRUE` is strict.
        format!("({expr})::boolean IS TRUE")
    }

    fn last_path_segment(&self, s: &str) -> String {
        // POSIX regexp on PG: strip everything up to and including the last `/`.
        format!("regexp_replace({s}, '.*/', '')")
    }
}

// ============================================================================
// SQLite
// ============================================================================

/// SQLite JSON1 dialect.
#[derive(Debug, Default, Clone, Copy)]
pub struct SqliteDialect;

impl Dialect for SqliteDialect {
    fn name(&self) -> &'static str {
        "sqlite"
    }

    fn placeholder(&self, idx: usize) -> String {
        format!("?{idx}")
    }

    fn string_literal(&self, s: &str) -> String {
        sqlite_string_literal(s)
    }

    fn json_field(&self, base: &str, key: &str) -> String {
        // A field accessor always addresses an object member, even when the
        // key is all digits (`json_path` would read that as an array index).
        let mut path = String::from("$");
        push_sqlite_member(&mut path, key);
        format!("json_extract({base}, {})", sqlite_string_literal(&path))
    }

    fn json_field_text(&self, base: &str, key: &str) -> String {
        // SQLite's json_extract returns the natural type; for object/array
        // values it returns JSON text. For scalar leaves callers usually want
        // the value directly — same call site.
        self.json_field(base, key)
    }

    fn json_path(&self, base: &str, segments: &[&str]) -> String {
        // SQLite JSON1 paths use `[N]` for array indices and `.field` for
        // object members. Numeric-only segments are array indices and must
        // not be preceded by a dot.
        format!(
            "json_extract({base}, {})",
            sqlite_json_path_literal(segments)
        )
    }

    fn json_path_text(&self, base: &str, segments: &[&str]) -> String {
        self.json_path(base, segments)
    }

    fn unnest_array(&self, expr: &str) -> String {
        format!("json_each({expr})")
    }

    fn coalesce_array(&self, expr: &str) -> String {
        format!("coalesce({expr}, '[]')")
    }

    fn json_type(&self, expr: &str) -> String {
        format!("json_type({expr})")
    }

    fn json_agg(&self, expr: &str) -> String {
        format!("json_group_array({expr})")
    }

    fn string_agg(&self, expr: &str, sep_param: &str) -> String {
        format!("group_concat({expr}, {sep_param})")
    }

    fn bool_true(&self) -> &'static str {
        "1"
    }

    fn bool_false(&self) -> &'static str {
        "0"
    }

    fn lateral_keyword(&self) -> &'static str {
        ""
    }

    fn cast(&self, inner: &str, ty: SqlType) -> String {
        match ty {
            SqlType::Text => format!("CAST({inner} AS TEXT)"),
            SqlType::Integer => format!("CAST({inner} AS INTEGER)"),
            SqlType::Decimal => format!("CAST({inner} AS REAL)"),
            // Boolean column projections — emit `'true'`/`'false'` text so the
            // runner's row mapper deserializes them as JSON booleans rather
            // than the JSON-number 1/0 it would get from CAST AS INTEGER.
            SqlType::Boolean => {
                format!("CASE WHEN ({inner}) THEN 'true' WHEN NOT ({inner}) THEN 'false' END")
            }
            SqlType::Json => format!("json({inner})"),
        }
    }

    fn has_json_type(&self, expr: &str, ty: JsonType) -> String {
        let name = match ty {
            JsonType::Object => "object",
            JsonType::Array => "array",
            JsonType::String => "text",
            JsonType::Number => "integer", // also "real"; callers needing both must compose
            JsonType::Boolean => "true",   // SQLite has no native boolean json_type
            JsonType::Null => "null",
        };
        format!("json_type({expr}) = '{name}'")
    }

    fn truthy_predicate(&self, expr: &str) -> String {
        // `json_extract` returns the JSON value's native SQLite type:
        // JSON booleans → integer 1/0, numbers → integer/real, strings → text.
        // Truthy is: non-NULL AND not zero/false. The explicit text-equality
        // check covers literal `'true'`/`'false'` text values just in case.
        format!("({expr}) IS NOT NULL AND ({expr}) != 0 AND ({expr}) != 'false'")
    }

    fn last_path_segment(&self, s: &str) -> String {
        // Calls the `fhir_last_segment` scalar UDF registered on every
        // pooled SQLite connection by the backend's connection initialiser
        // (see `crates/persistence/src/sof/sqlite_udfs.rs`).
        format!("fhir_last_segment({s})")
    }
}

/// Test-only scanner for the string literals in a SQL text, shared by the
/// dialect and emitter tests.
#[cfg(test)]
pub(super) mod test_support {
    /// A SQL text with its string literals picked out.
    pub(in crate::sof) struct Scanned {
        /// The SQL with every string literal (and an `E` prefix) replaced by
        /// `<lit>`: what is left is the SQL a literal could not have altered.
        pub skeleton: String,
        /// The decoded value of each string literal, in order of appearance.
        pub literals: Vec<String>,
    }

    /// Strings with a quote, a backslash, and both, plus the plain ones that
    /// must not change: `(value, SQLite literal, PostgreSQL literal)`.
    pub(in crate::sof) const STRING_LITERAL_CASES: &[(&str, &str, &str)] = &[
        // Plain strings render exactly as the old hand-built `'...'` did.
        ("", "''", "''"),
        ("male", "'male'", "'male'"),
        ("a b/c%", "'a b/c%'", "'a b/c%'"),
        // A quote is doubled in both dialects.
        ("a'b", "'a''b'", "'a''b'"),
        ("it's", "'it''s'", "'it''s'"),
        ("'", "''''", "''''"),
        // A backslash is literal in SQLite; PostgreSQL emits an escape string
        // so the value does not depend on `standard_conforming_strings`.
        ("a\\b", "'a\\b'", "E'a\\\\b'"),
        ("\\", "'\\'", "E'\\\\'"),
        // Both.
        ("it's a\\b", "'it''s a\\b'", "E'it''s a\\\\b'"),
        ("a\\'b", "'a\\''b'", "E'a\\\\''b'"),
    ];

    /// Scans `sql` for single-quoted string literals and decodes them.
    ///
    /// `plain_backslash_escapes` models PostgreSQL with
    /// `standard_conforming_strings = off`, where a backslash inside an
    /// ordinary `'...'` literal escapes the next character. `E'...'` literals
    /// always do. SQLite, and PostgreSQL by default, read a backslash in an
    /// ordinary literal as itself. Only `\\` and `\'` are accepted as escapes,
    /// since those are the only ones the emitter produces.
    ///
    /// Panics on an unterminated literal, which is exactly what an unescaped
    /// quote in an embedded string would cause.
    pub(in crate::sof) fn scan_literals(sql: &str, plain_backslash_escapes: bool) -> Scanned {
        let chars: Vec<char> = sql.chars().collect();
        let mut skeleton = String::new();
        let mut literals = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] != '\'' {
                skeleton.push(chars[i]);
                i += 1;
                continue;
            }
            let escape_string = i > 0
                && chars[i - 1] == 'E'
                && (i < 2 || !(chars[i - 2].is_alphanumeric() || chars[i - 2] == '_'));
            if escape_string {
                skeleton.pop(); // the `E` prefix belongs to the literal
            }
            let backslash_escapes = escape_string || plain_backslash_escapes;
            i += 1;
            let mut body = String::new();
            loop {
                assert!(i < chars.len(), "unterminated string literal in:\n{sql}");
                match chars[i] {
                    '\\' if backslash_escapes => {
                        let next = chars.get(i + 1).copied();
                        assert!(
                            matches!(next, Some('\\' | '\'')),
                            "unexpected escape {next:?} in:\n{sql}"
                        );
                        body.push(next.unwrap());
                        i += 2;
                    }
                    '\'' if chars.get(i + 1) == Some(&'\'') => {
                        body.push('\'');
                        i += 2;
                    }
                    '\'' => {
                        i += 1;
                        break;
                    }
                    c => {
                        body.push(c);
                        i += 1;
                    }
                }
            }
            skeleton.push_str("<lit>");
            literals.push(body);
        }
        Scanned { skeleton, literals }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{STRING_LITERAL_CASES, scan_literals};
    use super::*;

    #[test]
    fn pg_field_text() {
        assert_eq!(PgDialect.json_field_text("r.data", "id"), "r.data->>'id'");
    }

    #[test]
    fn pg_path_text_dotted() {
        assert_eq!(
            PgDialect.json_path_text("r.data", &["subject", "reference"]),
            "r.data#>>'{subject,reference}'"
        );
    }

    #[test]
    fn sqlite_field() {
        assert_eq!(
            SqliteDialect.json_field("r.data", "id"),
            "json_extract(r.data, '$.id')"
        );
    }

    #[test]
    fn sqlite_path_dotted() {
        assert_eq!(
            SqliteDialect.json_path("r.data", &["subject", "reference"]),
            "json_extract(r.data, '$.subject.reference')"
        );
    }

    #[test]
    fn placeholder_forms() {
        assert_eq!(PgDialect.placeholder(3), "$3");
        assert_eq!(SqliteDialect.placeholder(3), "?3");
    }

    #[test]
    fn plain_member_names() {
        for ok in ["id", "birthDate", "_birthDate", "valueQuantity", "a1", "_"] {
            assert!(is_plain_member_name(ok), "{ok:?} should be plain");
        }
        for bad in [
            "", "1a", "0", "a'b", "a\"b", "a b", "a.b", "a-b", "a,b", "a{b}", "a[0]", "a\\b",
            "a\0b", "é", "a)",
        ] {
            assert!(!is_plain_member_name(bad), "{bad:?} should not be plain");
        }
    }

    #[test]
    fn pg_key_is_quoted_and_escaped() {
        assert_eq!(
            PgDialect.json_field("r.data", "a'b"),
            "r.data->'a''b'",
            "a quote in the key is doubled"
        );
        assert_eq!(
            PgDialect.json_field_text("r.data", "x') OR 1=1 --"),
            "r.data->>'x'') OR 1=1 --'"
        );
        // Backslash: emitted as an E'' string so it is correct even with
        // standard_conforming_strings = off.
        assert_eq!(
            PgDialect.json_field("r.data", "a\\'b"),
            "r.data->E'a\\\\''b'"
        );
    }

    #[test]
    fn pg_path_array_quotes_special_elements() {
        // Plain elements (and numeric indices) stay bare.
        assert_eq!(
            PgDialect.json_path_text("r.data", &["name", "0", "family"]),
            "r.data#>>'{name,0,family}'"
        );
        // Special characters are double-quoted inside the array literal, and
        // the SQL layer doubles the single quote.
        assert_eq!(
            PgDialect.json_path("r.data", &["a b", "c,d"]),
            "r.data#>'{\"a b\",\"c,d\"}'"
        );
        assert_eq!(
            PgDialect.json_path("r.data", &["x}'", "y"]),
            "r.data#>'{\"x}''\",y}'"
        );
        // Embedded double quote / backslash are backslash-escaped in the
        // array literal; the backslash forces the E'' form.
        assert_eq!(
            PgDialect.json_path("r.data", &["a\"b", "c"]),
            "r.data#>E'{\"a\\\\\"b\",c}'"
        );
        // Empty and `null` elements must be quoted or the array is invalid /
        // reads as SQL NULL.
        assert_eq!(
            PgDialect.json_path("r.data", &["", "null"]),
            "r.data#>'{\"\",\"null\"}'"
        );
    }

    #[test]
    fn sqlite_field_is_quoted_and_escaped() {
        assert_eq!(
            SqliteDialect.json_field("r.data", "a'b"),
            "json_extract(r.data, '$.\"a''b\"')"
        );
        assert_eq!(
            SqliteDialect.json_field("r.data", "x.y"),
            "json_extract(r.data, '$.\"x.y\"')"
        );
        assert_eq!(
            SqliteDialect.json_field("r.data", "x') OR 1=1 --"),
            "json_extract(r.data, '$.\"x'') OR 1=1 --\"')"
        );
        // An embedded double quote or backslash is backslash-escaped inside
        // the quoted label so it cannot close the label early.
        assert_eq!(
            SqliteDialect.json_field("r.data", "a\"b\\c"),
            "json_extract(r.data, '$.\"a\\\"b\\\\c\"')"
        );
        // A field accessor never reads an all-digit key as an array index.
        assert_eq!(
            SqliteDialect.json_field("r.data", "0"),
            "json_extract(r.data, '$.\"0\"')"
        );
    }

    #[test]
    fn sqlite_path_quotes_special_segments_and_keeps_indices() {
        assert_eq!(
            SqliteDialect.json_path("r.data", &["name", "0", "family"]),
            "json_extract(r.data, '$.name[0].family')"
        );
        assert_eq!(
            SqliteDialect.json_path("r.data", &["a b", "0", "c'd"]),
            "json_extract(r.data, '$.\"a b\"[0].\"c''d\"')"
        );
    }

    /// Runs the dialect's output on a real SQLite, proving the quoted labels
    /// address the intended keys and that hostile names are inert there.
    #[cfg(feature = "sqlite")]
    #[test]
    fn sqlite_quoted_labels_address_the_intended_keys() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let doc = sqlite_string_literal(
            r#"{"a'b": 1, "x.y": 2, "a b": 3, "0": 4, "list": [{"a'b": 5}], "n": {"x.y": 6}}"#,
        );
        let eval = |expr: String| -> Option<i64> {
            conn.query_row(&format!("SELECT {expr}"), [], |r| r.get(0))
                .unwrap_or_else(|e| panic!("{expr}: {e}"))
        };
        let d = SqliteDialect;
        assert_eq!(eval(d.json_field(&doc, "a'b")), Some(1));
        assert_eq!(eval(d.json_field(&doc, "x.y")), Some(2));
        assert_eq!(eval(d.json_field(&doc, "a b")), Some(3));
        assert_eq!(eval(d.json_field(&doc, "0")), Some(4));
        assert_eq!(eval(d.json_path(&doc, &["list", "0", "a'b"])), Some(5));
        assert_eq!(eval(d.json_path(&doc, &["n", "x.y"])), Some(6));
        // Keys that do not exist (or that try to inject) are just NULL.
        assert_eq!(eval(d.json_field(&doc, "x') OR 1=1 --")), None);
        assert_eq!(eval(d.json_field(&doc, "a\"b")), None);
        assert_eq!(eval(d.json_field(&doc, "a\\b")), None);
        assert_eq!(eval(d.json_path(&doc, &["n", "a'b"])), None);
    }

    #[test]
    fn string_literals_escape_quotes() {
        assert_eq!(sqlite_string_literal("it's"), "'it''s'");
        assert_eq!(sqlite_string_literal("a\\b"), "'a\\b'");
        assert_eq!(pg_string_literal("it's"), "'it''s'");
        assert_eq!(pg_string_literal("a\\b"), "E'a\\\\b'");
        // NUL is dropped rather than truncating or breaking the statement.
        assert_eq!(sqlite_string_literal("a\0b"), "'ab'");
        assert_eq!(pg_string_literal("a\0b"), "'ab'");
    }

    #[test]
    fn string_literal_golden() {
        for (value, sqlite, pg) in STRING_LITERAL_CASES {
            assert_eq!(
                SqliteDialect.string_literal(value),
                *sqlite,
                "sqlite {value:?}"
            );
            assert_eq!(PgDialect.string_literal(value), *pg, "postgres {value:?}");
        }
    }

    #[test]
    fn string_literal_is_unchanged_for_strings_without_a_backslash() {
        // The hand-built form this method replaced.
        let legacy = |s: &str| format!("'{}'", s.replace('\'', "''"));
        for value in [
            "",
            "male",
            "a'b",
            "it's",
            "''",
            "a b/c%",
            "x_y",
            "é\u{1F600}",
        ] {
            assert_eq!(SqliteDialect.string_literal(value), legacy(value));
            assert_eq!(PgDialect.string_literal(value), legacy(value));
        }
        // SQLite never changes, backslash or not.
        for value in ["a\\b", "\\", "it's a\\b", "a\\'b"] {
            assert_eq!(SqliteDialect.string_literal(value), legacy(value));
        }
    }

    #[test]
    fn string_literal_decodes_to_the_value_under_both_pg_settings() {
        // With `standard_conforming_strings = off` a backslash inside an
        // ordinary `'...'` literal is an escape. Decoding the PostgreSQL
        // output under both settings must give back the original value.
        for (value, _, _) in STRING_LITERAL_CASES {
            let pg = PgDialect.string_literal(value);
            for backslash_escapes in [false, true] {
                let scanned = scan_literals(&pg, backslash_escapes);
                assert_eq!(
                    scanned.literals,
                    [value.to_string()],
                    "{pg} decoded with backslash escapes = {backslash_escapes}"
                );
            }
            let scanned = scan_literals(&SqliteDialect.string_literal(value), false);
            assert_eq!(scanned.literals, [value.to_string()]);
        }
    }

    /// The scanner has to model `standard_conforming_strings` faithfully, or
    /// the decoding tests above would pass vacuously.
    #[test]
    fn scanner_models_standard_conforming_strings() {
        // An ordinary literal: a backslash is literal when the setting is on
        // and an escape when it is off, so the same text decodes differently.
        assert_eq!(scan_literals(r"'a\\b'", false).literals, [r"a\\b"]);
        assert_eq!(scan_literals(r"'a\\b'", true).literals, [r"a\b"]);
        // An escape string always treats the backslash as an escape.
        assert_eq!(scan_literals(r"E'a\\b'", false).literals, [r"a\b"]);
        assert_eq!(scan_literals(r"E'a\\b'", true).literals, [r"a\b"]);
        // The `E` is part of the literal, not of the SQL around it.
        assert_eq!(scan_literals(r"x = E'a'", false).skeleton, "x = <lit>");
        // An identifier ending in `E` is not an escape-string prefix.
        assert_eq!(scan_literals(r"CASE'a\b'", false).literals, [r"a\b"]);
    }

    /// Runs the literals on a real SQLite: each must come back as the value.
    #[cfg(feature = "sqlite")]
    #[test]
    fn sqlite_string_literal_round_trips() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        for (value, _, _) in STRING_LITERAL_CASES {
            let sql = format!("SELECT {}", SqliteDialect.string_literal(value));
            let got: String = conn
                .query_row(&sql, [], |r| r.get(0))
                .unwrap_or_else(|e| panic!("{sql}: {e}"));
            assert_eq!(got, *value, "{sql}");
        }
    }
}
