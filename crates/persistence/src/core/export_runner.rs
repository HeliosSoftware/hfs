//! Tuning for the SQL-on-FHIR runner `$sql-export` jobs read through.
//!
//! An export job reads its whole result — every row of every subject, and
//! every SQLQuery dependency it materializes — and nothing about it is
//! interactive. Running it on the request-serving pool has two costs:
//!
//! - **It occupies request connections.** A multi-minute export statement
//!   holds a pooled connection for its whole life.
//! - **It runs under request-shaped settings.** The planner settings that suit
//!   a point read are not the ones that suit a full scan of a TOASTed document
//!   per resource. In particular, once an export statement detoasts each
//!   resource's document once into a computed column (the detoast-once SQL
//!   shape), the planner has no statistics on that column, memoizes on it,
//!   and every lookup misses: a `forEach` export in that shape measured 3.7x
//!   faster with `enable_memoize = off`. Statements that read `r.data`
//!   directly plan no Memoize node there, so the setting does not change them.
//!
//! [`ExportRunnerOptions`] carries the settings a backend may apply to a
//! separate, export-only runner. Backends without such a runner ignore it —
//! [`ResourceStorage::export_sof_runner`](crate::core::ResourceStorage::export_sof_runner)
//! defaults to the ordinary [`sof_runner`](crate::core::ResourceStorage::sof_runner).

use std::fmt;

/// Default number of export connections: `HFS_EXPORT_MAX_CONCURRENCY`'s
/// default, so each concurrently running job has one connection.
const DEFAULT_EXPORT_MAX_CONNECTIONS: usize = 4;

/// Settings for the runner `$sql-export` jobs read through.
///
/// Only the PostgreSQL backend acts on these today; every other backend
/// returns its ordinary runner and ignores them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRunnerOptions {
    /// Connections in the export pool. An export job runs one statement at a
    /// time, so one connection per concurrently running job is enough;
    /// HFS defaults this to `HFS_EXPORT_MAX_CONCURRENCY`. Values below 1 are
    /// treated as 1.
    pub max_connections: usize,

    /// `work_mem` for export connections. `None` keeps the server's own value.
    pub work_mem: Option<PgMemorySize>,

    /// `statement_timeout` for export statements, in milliseconds (`0` = no
    /// limit). `None` inherits the main pool's statement timeout.
    pub statement_timeout_ms: Option<u64>,

    /// Whether the planner may use Memoize nodes in export statements.
    ///
    /// `false` (the default) opens every export connection with
    /// `enable_memoize = off`; `true` leaves the server's own setting alone.
    /// The GUC exists from PostgreSQL 14; against an older server the backend
    /// leaves it out rather than failing every export connection.
    pub enable_memoize: bool,
}

impl Default for ExportRunnerOptions {
    fn default() -> Self {
        Self {
            max_connections: DEFAULT_EXPORT_MAX_CONNECTIONS,
            work_mem: None,
            statement_timeout_ms: None,
            enable_memoize: false,
        }
    }
}

/// Unit of a [`PgMemorySize`], as PostgreSQL spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PgMemoryUnit {
    /// Bytes (`B`).
    Bytes,
    /// Kilobytes (`kB`). PostgreSQL's unit for a bare number.
    Kilobytes,
    /// Megabytes (`MB`).
    Megabytes,
    /// Gigabytes (`GB`).
    Gigabytes,
    /// Terabytes (`TB`).
    Terabytes,
}

impl PgMemoryUnit {
    /// The unit literal PostgreSQL accepts.
    fn as_guc(self) -> &'static str {
        match self {
            Self::Bytes => "B",
            Self::Kilobytes => "kB",
            Self::Megabytes => "MB",
            Self::Gigabytes => "GB",
            Self::Terabytes => "TB",
        }
    }

    fn bytes(self) -> u128 {
        match self {
            Self::Bytes => 1,
            Self::Kilobytes => 1 << 10,
            Self::Megabytes => 1 << 20,
            Self::Gigabytes => 1 << 30,
            Self::Terabytes => 1 << 40,
        }
    }
}

/// A validated PostgreSQL memory setting such as `work_mem`.
///
/// Parsed from operator text, but only ever rendered from its parts — an
/// integer and a fixed unit literal — so nothing operator-supplied reaches a
/// connection's startup packet verbatim. The range is `work_mem`'s own
/// (64 kB to 2,147,483,647 kB), checked here so a bad value fails at startup
/// instead of on every export's first connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PgMemorySize {
    amount: u64,
    unit: PgMemoryUnit,
}

impl PgMemorySize {
    /// `work_mem`'s lower bound, in kB.
    const MIN_KB: u128 = 64;
    /// `work_mem`'s upper bound, in kB (`INT_MAX`).
    const MAX_KB: u128 = i32::MAX as u128;

    /// Parses an integer followed by an optional unit — `B`, `kB`, `MB`,
    /// `GB` or `TB`, matched case-insensitively, with optional whitespace in
    /// between. A bare number is kilobytes, as in `postgresql.conf`.
    ///
    /// # Errors
    ///
    /// A message naming the expected form when the text is not an integer and
    /// a unit, or when the size is outside 64 kB ..= 2,147,483,647 kB.
    pub fn parse(text: &str) -> Result<Self, String> {
        let trimmed = text.trim();
        let digits_end = trimmed
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(trimmed.len());
        let (digits, unit) = trimmed.split_at(digits_end);
        let invalid = || {
            format!(
                "'{text}' is not a PostgreSQL memory size: expected an integer with an \
                 optional unit B, kB, MB, GB or TB (for example 64MB)"
            )
        };
        if digits.is_empty() {
            return Err(invalid());
        }
        let unit = match unit.trim().to_ascii_lowercase().as_str() {
            "" | "kb" => PgMemoryUnit::Kilobytes,
            "b" => PgMemoryUnit::Bytes,
            "mb" => PgMemoryUnit::Megabytes,
            "gb" => PgMemoryUnit::Gigabytes,
            "tb" => PgMemoryUnit::Terabytes,
            _ => return Err(invalid()),
        };
        let amount: u64 = digits.parse().map_err(|_| invalid())?;
        let size = Self { amount, unit };
        let bytes = size.bytes();
        if !(Self::MIN_KB * 1024..=Self::MAX_KB * 1024).contains(&bytes) {
            return Err(format!(
                "'{text}' is outside PostgreSQL's work_mem range of 64kB to 2147483647kB"
            ));
        }
        Ok(size)
    }

    /// The size in bytes. `u128`, so no amount/unit pair can overflow.
    pub fn bytes(&self) -> u128 {
        u128::from(self.amount) * self.unit.bytes()
    }
}

impl fmt::Display for PgMemorySize {
    /// The canonical GUC form, e.g. `64MB`: digits and a unit, no whitespace.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.amount, self.unit.as_guc())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_sizes_render_canonically() {
        for (input, rendered) in [
            ("64MB", "64MB"),
            (" 64 mb ", "64MB"),
            ("65536", "65536kB"),
            ("256kB", "256kB"),
            ("1gb", "1GB"),
            ("65536B", "65536B"),
            ("1TB", "1TB"),
        ] {
            let size = PgMemorySize::parse(input).unwrap_or_else(|e| panic!("{input}: {e}"));
            assert_eq!(size.to_string(), rendered, "{input}");
        }
    }

    #[test]
    fn memory_sizes_outside_work_mem_range_are_rejected() {
        // 64 kB is the floor, 2,147,483,647 kB (just under 2 TB) the ceiling.
        assert!(PgMemorySize::parse("64kB").is_ok());
        assert!(PgMemorySize::parse("63kB").is_err());
        assert!(PgMemorySize::parse("65535B").is_err());
        assert!(PgMemorySize::parse("0").is_err());
        assert!(PgMemorySize::parse("2147483647kB").is_ok());
        assert!(PgMemorySize::parse("2147483648kB").is_err());
        assert!(PgMemorySize::parse("2TB").is_err());
        // An amount too large for u64 is a parse error, not an overflow.
        assert!(PgMemorySize::parse("99999999999999999999999MB").is_err());
    }

    #[test]
    fn nothing_but_an_integer_and_a_unit_parses() {
        // Every one of these would otherwise reach the startup packet: a
        // second `-c`, a quote, a decimal or a sign is refused, not escaped.
        for input in [
            "",
            "MB",
            "64MB -c enable_seqscan=off",
            "64MB\\ x",
            "'64MB'",
            "1.5GB",
            "-64MB",
            "+64MB",
            "64 megabytes",
            "64MiB",
        ] {
            let err = PgMemorySize::parse(input).unwrap_err();
            assert!(err.contains("memory size"), "{input}: {err}");
        }
    }

    #[test]
    fn default_options_turn_memoize_off_and_inherit_the_rest() {
        let options = ExportRunnerOptions::default();
        assert_eq!(options.max_connections, 4);
        assert_eq!(options.work_mem, None);
        assert_eq!(options.statement_timeout_ms, None);
        assert!(!options.enable_memoize);
    }
}
