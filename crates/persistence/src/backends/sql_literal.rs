//! Rendering strings as SQL string literals for the SQLite and PostgreSQL
//! query builders.
//!
//! The builders bind search *values* as parameters, but splice some strings
//! into the SQL text: search parameter names, resource types, implicit token
//! systems and cursor values. Several of those reach the builder from the
//! request (`_revinclude=Type:name`, a stored SearchParameter's `code`), so
//! none may be interpolated as-is. Every such splice goes through
//! [`sql_string_literal`].

/// Renders `value` as a single-quoted SQL string literal, doubling any single
/// quote inside it.
///
/// A value containing a backslash or a NUL renders as `NULL` instead, which
/// compares equal to nothing, so the predicate it is part of matches no rows.
/// Doubling quotes is only a complete escape where backslash is not an escape
/// character: always in SQLite, and in PostgreSQL while
/// `standard_conforming_strings` is on (the default). Failing closed keeps the
/// escape correct without depending on that setting. No search parameter
/// name, resource type, id or token system legitimately contains either
/// character.
pub(crate) fn sql_string_literal(value: &str) -> String {
    if value.contains(['\\', '\0']) {
        return "NULL".to_string();
    }
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::sql_string_literal;

    #[test]
    fn quotes_and_doubles_single_quotes() {
        assert_eq!(sql_string_literal("identifier"), "'identifier'");
        assert_eq!(sql_string_literal("ev'il"), "'ev''il'");
        assert_eq!(sql_string_literal("'"), "''''");
        assert_eq!(sql_string_literal(""), "''");
    }

    #[test]
    fn backslash_and_nul_fail_closed() {
        assert_eq!(sql_string_literal("a\\' OR 1=1 --"), "NULL");
        assert_eq!(sql_string_literal("a\0b"), "NULL");
    }
}
