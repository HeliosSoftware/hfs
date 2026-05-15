//! Dialect trait — token-level SQL emission for PostgreSQL JSONB and SQLite JSON1.
//!
//! The compiler builds dialect-independent IR ([`PlanNode`](super::ir::PlanNode)
//! and [`SqlExpr`](super::ir::SqlExpr)); the emitter walks the IR and asks the
//! dialect for each concrete SQL token. Keeping these helpers behind a trait
//! confines per-dialect divergence (operator syntax, parameter form, JSON
//! function names) to two small implementations.

#![allow(dead_code)] // Stage 1 scaffold; consumers land in stages 2–5.
#![allow(missing_docs)] // Per-method docs land alongside their consumers in stages 2–5.

use super::ir::{JsonType, SqlType};

/// Per-dialect SQL emission helpers.
pub trait Dialect: Send + Sync {
    /// Short identifier for diagnostics ("postgres", "sqlite").
    fn name(&self) -> &'static str;

    /// Render a 1-based parameter placeholder (`$1` for PG, `?1` for SQLite).
    fn placeholder(&self, idx: usize) -> String;

    /// `base->'key'` (returns JSON value).
    fn json_field(&self, base: &str, key: &str) -> String;

    /// `base->>'key'` (returns text).
    fn json_field_text(&self, base: &str, key: &str) -> String;

    /// Multi-key path returning a JSON value.
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

    /// SQL boolean literals.
    fn bool_true(&self) -> &'static str;
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

    fn json_field(&self, base: &str, key: &str) -> String {
        format!("{base}->'{key}'")
    }

    fn json_field_text(&self, base: &str, key: &str) -> String {
        format!("{base}->>'{key}'")
    }

    fn json_path(&self, base: &str, segments: &[&str]) -> String {
        if segments.len() == 1 {
            self.json_field(base, segments[0])
        } else {
            format!("{base}#>'{{{}}}'", segments.join(","))
        }
    }

    fn json_path_text(&self, base: &str, segments: &[&str]) -> String {
        if segments.len() == 1 {
            self.json_field_text(base, segments[0])
        } else {
            format!("{base}#>>'{{{}}}'", segments.join(","))
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

    fn json_field(&self, base: &str, key: &str) -> String {
        format!("json_extract({base}, '$.{key}')")
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
        let mut path = String::from("$");
        for seg in segments {
            if seg.chars().all(|c| c.is_ascii_digit()) {
                path.push('[');
                path.push_str(seg);
                path.push(']');
            } else {
                path.push('.');
                path.push_str(seg);
            }
        }
        format!("json_extract({base}, '{path}')")
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

#[cfg(test)]
mod tests {
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
}
