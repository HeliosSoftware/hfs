//! SQLite scalar UDFs registered for the in-DB SOF runner.
//!
//! These functions cover SQL operations that the SoF v2 conformance suite
//! exercises but that don't have a clean built-in equivalent in SQLite's JSON1
//! /core dialect (no regex, no `split_part`, no `last_value`-of-string). The
//! UDFs are registered on every pooled connection at acquire time via the
//! `SqliteConnectionManager::with_init` callback wired in
//! `backends::sqlite::backend`.

use rusqlite::Connection;
use rusqlite::functions::FunctionFlags;

/// Registers the SOF helper UDFs on `conn`.
///
/// Currently:
/// - `fhir_last_segment(text) -> text` — substring of the input after the
///   last `/`, used by `getReferenceKey()` to extract the id portion of a
///   `Reference.reference` like `Patient/123`. Returns the input unchanged
///   when no `/` is present, and NULL when the input is NULL.
pub fn register(conn: &Connection) -> rusqlite::Result<()> {
    conn.create_scalar_function(
        "fhir_last_segment",
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let arg = ctx.get_raw(0);
            let s = match arg {
                rusqlite::types::ValueRef::Null => return Ok(None::<String>),
                rusqlite::types::ValueRef::Text(t) => std::str::from_utf8(t)
                    .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))?,
                _ => {
                    return Err(rusqlite::Error::UserFunctionError(
                        "fhir_last_segment expects a text argument".into(),
                    ));
                }
            };
            Ok(Some(match s.rfind('/') {
                Some(idx) => s[idx + 1..].to_string(),
                None => s.to_string(),
            }))
        },
    )?;
    Ok(())
}
