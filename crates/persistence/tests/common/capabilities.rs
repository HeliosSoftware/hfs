//! Capability matrix for backend testing.
//!
//! This module defines the [`CapabilityMatrix`] which maps backends to their
//! support levels for each capability. This drives test execution decisions.

use std::collections::HashMap;

use helios_persistence::core::{BackendCapability, BackendKind};
use serde::{Deserialize, Serialize};

/// Support level for a capability on a specific backend.
///
/// This enum determines how tests should behave for each capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SupportLevel {
    /// Fully implemented - run test and expect full compliance.
    Implemented,
    /// Partially implemented - run test but allow partial failures.
    Partial,
    /// Planned but not yet implemented - skip test.
    Planned,
    /// Not applicable to this backend - skip test.
    NotPlanned,
    /// Requires an external service (e.g., terminology server) - skip unless available.
    RequiresExternalService,
}

impl SupportLevel {
    /// Returns true if tests should be run for this support level.
    pub fn should_run(&self) -> bool {
        matches!(self, SupportLevel::Implemented | SupportLevel::Partial)
    }

    /// Returns true if partial failures should be allowed.
    pub fn allows_partial(&self) -> bool {
        matches!(self, SupportLevel::Partial)
    }

    /// Returns true if the test should be skipped.
    pub fn should_skip(&self) -> bool {
        !self.should_run()
    }
}

impl Default for SupportLevel {
    fn default() -> Self {
        SupportLevel::Planned
    }
}

impl std::fmt::Display for SupportLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let symbol = match self {
            SupportLevel::Implemented => "✓",
            SupportLevel::Partial => "◐",
            SupportLevel::Planned => "○",
            SupportLevel::NotPlanned => "✗",
            SupportLevel::RequiresExternalService => "†",
        };
        write!(f, "{}", symbol)
    }
}

/// Matrix mapping backends to capability support levels.
///
/// This is the source of truth for what each backend supports. Tests use this
/// to determine whether to run, skip, or expect partial results.
///
/// # Example
///
/// ```ignore
/// let matrix = CapabilityMatrix::default();
/// let level = matrix.support_level(BackendKind::Sqlite, BackendCapability::Crud);
/// assert_eq!(level, SupportLevel::Implemented);
/// ```
#[derive(Debug, Clone)]
pub struct CapabilityMatrix {
    /// Map of backend → capability → support level.
    matrix: HashMap<BackendKind, HashMap<BackendCapability, SupportLevel>>,
}

impl CapabilityMatrix {
    /// Creates a new empty capability matrix.
    pub fn new() -> Self {
        Self {
            matrix: HashMap::new(),
        }
    }

    /// Creates the default capability matrix based on the design document.
    ///
    /// This includes all known backends and their support levels as defined
    /// in the persistence layer README.  Keep this code in sync with the README!
    pub fn default() -> Self {
        let mut matrix = Self::new();

        // SQLite capabilities
        matrix.set_backend_capabilities(
            BackendKind::Sqlite,
            vec![
                (BackendCapability::Crud, SupportLevel::Implemented),
                (BackendCapability::Versioning, SupportLevel::Implemented),
                (BackendCapability::InstanceHistory, SupportLevel::Implemented),
                (BackendCapability::TypeHistory, SupportLevel::Implemented),
                (BackendCapability::SystemHistory, SupportLevel::Implemented),
                (BackendCapability::BasicSearch, SupportLevel::Implemented),
                (BackendCapability::DateSearch, SupportLevel::Implemented),
                (BackendCapability::ReferenceSearch, SupportLevel::Implemented),
                (BackendCapability::ChainedSearch, SupportLevel::Partial),
                (BackendCapability::ReverseChaining, SupportLevel::Partial),
                (BackendCapability::Include, SupportLevel::Implemented),
                (BackendCapability::Revinclude, SupportLevel::Implemented),
                (BackendCapability::FullTextSearch, SupportLevel::NotPlanned),
                (BackendCapability::TerminologySearch, SupportLevel::RequiresExternalService),
                (BackendCapability::Transactions, SupportLevel::Implemented),
                (BackendCapability::OptimisticLocking, SupportLevel::Implemented),
                (BackendCapability::CursorPagination, SupportLevel::Planned),
                (BackendCapability::OffsetPagination, SupportLevel::Implemented),
                (BackendCapability::Sorting, SupportLevel::Implemented),
                (BackendCapability::BulkExport, SupportLevel::Planned),
                (BackendCapability::SharedSchema, SupportLevel::Implemented),
                (BackendCapability::SchemaPerTenant, SupportLevel::NotPlanned),
                (BackendCapability::DatabasePerTenant, SupportLevel::NotPlanned),
            ],
        );

        // PostgreSQL capabilities
        matrix.set_backend_capabilities(
            BackendKind::Postgres,
            vec![
                (BackendCapability::Crud, SupportLevel::Implemented),
                (BackendCapability::Versioning, SupportLevel::Implemented),
                (BackendCapability::InstanceHistory, SupportLevel::Implemented),
                (BackendCapability::TypeHistory, SupportLevel::Implemented),
                (BackendCapability::SystemHistory, SupportLevel::Implemented),
                (BackendCapability::BasicSearch, SupportLevel::Implemented),
                (BackendCapability::DateSearch, SupportLevel::Implemented),
                (BackendCapability::ReferenceSearch, SupportLevel::Implemented),
                (BackendCapability::ChainedSearch, SupportLevel::Partial),
                (BackendCapability::ReverseChaining, SupportLevel::Partial),
                (BackendCapability::Include, SupportLevel::Implemented),
                (BackendCapability::Revinclude, SupportLevel::Implemented),
                (BackendCapability::FullTextSearch, SupportLevel::Partial),
                (BackendCapability::TerminologySearch, SupportLevel::RequiresExternalService),
                (BackendCapability::Transactions, SupportLevel::Implemented),
                (BackendCapability::OptimisticLocking, SupportLevel::Implemented),
                (BackendCapability::CursorPagination, SupportLevel::Planned),
                (BackendCapability::OffsetPagination, SupportLevel::Implemented),
                (BackendCapability::Sorting, SupportLevel::Implemented),
                (BackendCapability::BulkExport, SupportLevel::Planned),
                (BackendCapability::SharedSchema, SupportLevel::Implemented),
                (BackendCapability::SchemaPerTenant, SupportLevel::Planned),
                (BackendCapability::DatabasePerTenant, SupportLevel::Planned),
            ],
        );

        // MongoDB capabilities
        matrix.set_backend_capabilities(
            BackendKind::MongoDB,
            vec![
                (BackendCapability::Crud, SupportLevel::Implemented),
                (BackendCapability::Versioning, SupportLevel::Implemented),
                (BackendCapability::InstanceHistory, SupportLevel::Implemented),
                (BackendCapability::TypeHistory, SupportLevel::Implemented),
                (BackendCapability::SystemHistory, SupportLevel::Implemented),
                (BackendCapability::BasicSearch, SupportLevel::Implemented),
                (BackendCapability::DateSearch, SupportLevel::Implemented),
                (BackendCapability::ReferenceSearch, SupportLevel::Implemented),
                (BackendCapability::ChainedSearch, SupportLevel::Planned),
                (BackendCapability::ReverseChaining, SupportLevel::Planned),
                (BackendCapability::Include, SupportLevel::Planned),
                (BackendCapability::Revinclude, SupportLevel::Planned),
                (BackendCapability::FullTextSearch, SupportLevel::Planned),
                (BackendCapability::TerminologySearch, SupportLevel::Planned),
                (BackendCapability::Transactions, SupportLevel::Implemented),
                (BackendCapability::OptimisticLocking, SupportLevel::Implemented),
                (BackendCapability::CursorPagination, SupportLevel::Implemented),
                (BackendCapability::OffsetPagination, SupportLevel::Implemented),
                (BackendCapability::Sorting, SupportLevel::Implemented),
                (BackendCapability::BulkExport, SupportLevel::Planned),
                (BackendCapability::SharedSchema, SupportLevel::Implemented),
                (BackendCapability::SchemaPerTenant, SupportLevel::NotPlanned),
                (BackendCapability::DatabasePerTenant, SupportLevel::Planned),
            ],
        );

        // Cassandra capabilities
        matrix.set_backend_capabilities(
            BackendKind::Cassandra,
            vec![
                (BackendCapability::Crud, SupportLevel::Planned),
                (BackendCapability::Versioning, SupportLevel::Planned),
                (BackendCapability::InstanceHistory, SupportLevel::Planned),
                (BackendCapability::TypeHistory, SupportLevel::NotPlanned),
                (BackendCapability::SystemHistory, SupportLevel::NotPlanned),
                (BackendCapability::BasicSearch, SupportLevel::Partial),
                (BackendCapability::DateSearch, SupportLevel::Planned),
                (BackendCapability::ReferenceSearch, SupportLevel::NotPlanned),
                (BackendCapability::ChainedSearch, SupportLevel::NotPlanned),
                (BackendCapability::ReverseChaining, SupportLevel::NotPlanned),
                (BackendCapability::Include, SupportLevel::NotPlanned),
                (BackendCapability::Revinclude, SupportLevel::NotPlanned),
                (BackendCapability::FullTextSearch, SupportLevel::NotPlanned),
                (BackendCapability::TerminologySearch, SupportLevel::NotPlanned),
                (BackendCapability::Transactions, SupportLevel::NotPlanned),
                (BackendCapability::OptimisticLocking, SupportLevel::Planned),
                (BackendCapability::CursorPagination, SupportLevel::Planned),
                (BackendCapability::OffsetPagination, SupportLevel::NotPlanned),
                (BackendCapability::Sorting, SupportLevel::NotPlanned),
                (BackendCapability::BulkExport, SupportLevel::Planned),
                (BackendCapability::SharedSchema, SupportLevel::Planned),
                (BackendCapability::SchemaPerTenant, SupportLevel::NotPlanned),
                (BackendCapability::DatabasePerTenant, SupportLevel::Planned),
            ],
        );

        // Neo4j capabilities
        matrix.set_backend_capabilities(
            BackendKind::Neo4j,
            vec![
                (BackendCapability::Crud, SupportLevel::Planned),
                (BackendCapability::Versioning, SupportLevel::Partial),
                (BackendCapability::InstanceHistory, SupportLevel::Partial),
                (BackendCapability::TypeHistory, SupportLevel::NotPlanned),
                (BackendCapability::SystemHistory, SupportLevel::NotPlanned),
                (BackendCapability::BasicSearch, SupportLevel::Planned),
                (BackendCapability::DateSearch, SupportLevel::Planned),
                (BackendCapability::ReferenceSearch, SupportLevel::Implemented),
                (BackendCapability::ChainedSearch, SupportLevel::Implemented),
                (BackendCapability::ReverseChaining, SupportLevel::Implemented),
                (BackendCapability::Include, SupportLevel::Implemented),
                (BackendCapability::Revinclude, SupportLevel::Implemented),
                (BackendCapability::FullTextSearch, SupportLevel::Partial),
                (BackendCapability::TerminologySearch, SupportLevel::Implemented),
                (BackendCapability::Transactions, SupportLevel::Planned),
                (BackendCapability::OptimisticLocking, SupportLevel::Partial),
                (BackendCapability::CursorPagination, SupportLevel::Planned),
                (BackendCapability::OffsetPagination, SupportLevel::Planned),
                (BackendCapability::Sorting, SupportLevel::Planned),
                (BackendCapability::BulkExport, SupportLevel::NotPlanned),
                (BackendCapability::SharedSchema, SupportLevel::Planned),
                (BackendCapability::SchemaPerTenant, SupportLevel::NotPlanned),
                (BackendCapability::DatabasePerTenant, SupportLevel::Planned),
            ],
        );

        // Elasticsearch capabilities
        matrix.set_backend_capabilities(
            BackendKind::Elasticsearch,
            vec![
                (BackendCapability::Crud, SupportLevel::Implemented),
                (BackendCapability::Versioning, SupportLevel::Partial),
                (BackendCapability::InstanceHistory, SupportLevel::NotPlanned),
                (BackendCapability::TypeHistory, SupportLevel::NotPlanned),
                (BackendCapability::SystemHistory, SupportLevel::NotPlanned),
                (BackendCapability::BasicSearch, SupportLevel::Implemented),
                (BackendCapability::DateSearch, SupportLevel::Implemented),
                (BackendCapability::ReferenceSearch, SupportLevel::Implemented),
                (BackendCapability::ChainedSearch, SupportLevel::NotPlanned),
                (BackendCapability::ReverseChaining, SupportLevel::NotPlanned),
                (BackendCapability::Include, SupportLevel::Implemented),
                (BackendCapability::Revinclude, SupportLevel::Implemented),
                (BackendCapability::FullTextSearch, SupportLevel::Implemented),
                (BackendCapability::TerminologySearch, SupportLevel::RequiresExternalService),
                (BackendCapability::Transactions, SupportLevel::NotPlanned),
                (BackendCapability::OptimisticLocking, SupportLevel::Implemented),
                (BackendCapability::CursorPagination, SupportLevel::Implemented),
                (BackendCapability::OffsetPagination, SupportLevel::Implemented),
                (BackendCapability::Sorting, SupportLevel::Implemented),
                (BackendCapability::BulkExport, SupportLevel::Implemented),
                (BackendCapability::SharedSchema, SupportLevel::Implemented),
                (BackendCapability::SchemaPerTenant, SupportLevel::NotPlanned),
                (BackendCapability::DatabasePerTenant, SupportLevel::Planned),
            ],
        );

        // S3 capabilities
        matrix.set_backend_capabilities(
            BackendKind::S3,
            vec![
                (BackendCapability::Crud, SupportLevel::Implemented),
                (BackendCapability::Versioning, SupportLevel::Implemented),
                (BackendCapability::InstanceHistory, SupportLevel::Implemented),
                (BackendCapability::TypeHistory, SupportLevel::Implemented),
                (BackendCapability::SystemHistory, SupportLevel::Implemented),
                (BackendCapability::BasicSearch, SupportLevel::NotPlanned),
                (BackendCapability::DateSearch, SupportLevel::NotPlanned),
                (BackendCapability::ReferenceSearch, SupportLevel::NotPlanned),
                (BackendCapability::ChainedSearch, SupportLevel::NotPlanned),
                (BackendCapability::ReverseChaining, SupportLevel::NotPlanned),
                (BackendCapability::Include, SupportLevel::NotPlanned),
                (BackendCapability::Revinclude, SupportLevel::NotPlanned),
                (BackendCapability::FullTextSearch, SupportLevel::NotPlanned),
                (BackendCapability::TerminologySearch, SupportLevel::NotPlanned),
                (BackendCapability::Transactions, SupportLevel::NotPlanned),
                (BackendCapability::OptimisticLocking, SupportLevel::Implemented),
                (BackendCapability::CursorPagination, SupportLevel::Implemented),
                (BackendCapability::OffsetPagination, SupportLevel::NotPlanned),
                (BackendCapability::Sorting, SupportLevel::NotPlanned),
                (BackendCapability::BulkExport, SupportLevel::Implemented),
                (BackendCapability::BulkImport, SupportLevel::Implemented),
                (BackendCapability::SharedSchema, SupportLevel::Implemented),
                (BackendCapability::SchemaPerTenant, SupportLevel::NotPlanned),
                (BackendCapability::DatabasePerTenant, SupportLevel::Implemented),
            ],
        );

        matrix
    }

    /// Sets capabilities for a backend.
    pub fn set_backend_capabilities(
        &mut self,
        backend: BackendKind,
        capabilities: Vec<(BackendCapability, SupportLevel)>,
    ) {
        let map = self.matrix.entry(backend).or_insert_with(HashMap::new);
        for (cap, level) in capabilities {
            map.insert(cap, level);
        }
    }

    /// Gets the support level for a capability on a backend.
    ///
    /// Returns `SupportLevel::Planned` if the combination is not explicitly set.
    pub fn support_level(&self, backend: BackendKind, capability: BackendCapability) -> SupportLevel {
        self.matrix
            .get(&backend)
            .and_then(|caps| caps.get(&capability))
            .copied()
            .unwrap_or(SupportLevel::Planned)
    }

    /// Checks if a backend supports a capability at any level that allows testing.
    pub fn can_test(&self, backend: BackendKind, capability: BackendCapability) -> bool {
        self.support_level(backend, capability).should_run()
    }

    /// Returns all capabilities that are implemented for a backend.
    pub fn implemented_capabilities(&self, backend: BackendKind) -> Vec<BackendCapability> {
        self.matrix
            .get(&backend)
            .map(|caps| {
                caps.iter()
                    .filter(|(_, level)| **level == SupportLevel::Implemented)
                    .map(|(cap, _)| *cap)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Returns all capabilities that have any test coverage for a backend.
    pub fn testable_capabilities(&self, backend: BackendKind) -> Vec<BackendCapability> {
        self.matrix
            .get(&backend)
            .map(|caps| {
                caps.iter()
                    .filter(|(_, level)| level.should_run())
                    .map(|(cap, _)| *cap)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Prints a summary of the capability matrix.
    pub fn print_summary(&self) {
        let all_capabilities = vec![
            BackendCapability::Crud,
            BackendCapability::Versioning,
            BackendCapability::InstanceHistory,
            BackendCapability::TypeHistory,
            BackendCapability::SystemHistory,
            BackendCapability::BasicSearch,
            BackendCapability::DateSearch,
            BackendCapability::ReferenceSearch,
            BackendCapability::ChainedSearch,
            BackendCapability::ReverseChaining,
            BackendCapability::Include,
            BackendCapability::Revinclude,
            BackendCapability::FullTextSearch,
            BackendCapability::TerminologySearch,
            BackendCapability::Transactions,
            BackendCapability::OptimisticLocking,
            BackendCapability::CursorPagination,
            BackendCapability::OffsetPagination,
            BackendCapability::Sorting,
            BackendCapability::BulkExport,
            BackendCapability::SharedSchema,
            BackendCapability::SchemaPerTenant,
            BackendCapability::DatabasePerTenant,
        ];

        let backends = vec![
            BackendKind::Sqlite,
            BackendKind::Postgres,
            BackendKind::MongoDB,
            BackendKind::Cassandra,
            BackendKind::Neo4j,
            BackendKind::Elasticsearch,
            BackendKind::S3,
        ];

        println!("\nCapability Matrix:");
        println!("Legend: {} Implemented, {} Partial, {} Planned, {} NotPlanned, {} RequiresExternalService\n",
            SupportLevel::Implemented,
            SupportLevel::Partial,
            SupportLevel::Planned,
            SupportLevel::NotPlanned,
            SupportLevel::RequiresExternalService,
        );

        // Header
        print!("{:<25}", "Capability");
        for backend in &backends {
            print!("{:>10}", format!("{}", backend));
        }
        println!();

        // Separator
        println!("{}", "-".repeat(25 + backends.len() * 10));

        // Rows
        for cap in &all_capabilities {
            print!("{:<25}", format!("{}", cap));
            for backend in &backends {
                let level = self.support_level(*backend, *cap);
                print!("{:>10}", format!("{}", level));
            }
            println!();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_support_level_should_run() {
        assert!(SupportLevel::Implemented.should_run());
        assert!(SupportLevel::Partial.should_run());
        assert!(!SupportLevel::Planned.should_run());
        assert!(!SupportLevel::NotPlanned.should_run());
        assert!(!SupportLevel::RequiresExternalService.should_run());
    }

    #[test]
    fn test_support_level_display() {
        assert_eq!(SupportLevel::Implemented.to_string(), "✓");
        assert_eq!(SupportLevel::Partial.to_string(), "◐");
        assert_eq!(SupportLevel::Planned.to_string(), "○");
        assert_eq!(SupportLevel::NotPlanned.to_string(), "✗");
        assert_eq!(SupportLevel::RequiresExternalService.to_string(), "†");
    }

    #[test]
    fn test_default_matrix() {
        let matrix = CapabilityMatrix::default();

        // SQLite should have CRUD implemented
        assert_eq!(
            matrix.support_level(BackendKind::Sqlite, BackendCapability::Crud),
            SupportLevel::Implemented
        );

        // SQLite should not have full-text search
        assert_eq!(
            matrix.support_level(BackendKind::Sqlite, BackendCapability::FullTextSearch),
            SupportLevel::NotPlanned
        );

        // Neo4j should have chained search
        assert_eq!(
            matrix.support_level(BackendKind::Neo4j, BackendCapability::ChainedSearch),
            SupportLevel::Implemented
        );
    }

    #[test]
    fn test_implemented_capabilities() {
        let matrix = CapabilityMatrix::default();
        let sqlite_caps = matrix.implemented_capabilities(BackendKind::Sqlite);

        assert!(sqlite_caps.contains(&BackendCapability::Crud));
        assert!(sqlite_caps.contains(&BackendCapability::Transactions));
        assert!(!sqlite_caps.contains(&BackendCapability::FullTextSearch));
    }

    #[test]
    fn test_testable_capabilities() {
        let matrix = CapabilityMatrix::default();
        let sqlite_caps = matrix.testable_capabilities(BackendKind::Sqlite);

        // Should include implemented
        assert!(sqlite_caps.contains(&BackendCapability::Crud));
        // Should include partial
        assert!(sqlite_caps.contains(&BackendCapability::ChainedSearch));
        // Should not include not planned
        assert!(!sqlite_caps.contains(&BackendCapability::FullTextSearch));
    }

    #[test]
    fn test_can_test() {
        let matrix = CapabilityMatrix::default();

        assert!(matrix.can_test(BackendKind::Sqlite, BackendCapability::Crud));
        assert!(!matrix.can_test(BackendKind::Sqlite, BackendCapability::FullTextSearch));
    }
}
