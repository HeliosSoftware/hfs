//! Search strategy capability for SQLite backend.
//!
//! Defines which search strategies the SQLite backend supports.

use crate::types::{IndexingMode, JsonbCapabilities, SearchStrategy};

/// Trait for backends to declare search strategy capabilities.
pub trait SearchStrategyCapability: Send + Sync {
    /// Does this backend support pre-computed indexes?
    fn supports_precomputed_index(&self) -> bool;

    /// Does this backend support query-time JSONB evaluation?
    fn supports_jsonb_evaluation(&self) -> bool;

    /// Get recommended strategy for this backend.
    fn recommended_strategy(&self) -> SearchStrategy;

    /// Get supported JSONB operators.
    fn jsonb_capabilities(&self) -> JsonbCapabilities;

    /// Get the current indexing mode.
    fn indexing_mode(&self) -> IndexingMode;
}

/// SQLite-specific search strategy implementation.
pub struct SqliteSearchStrategy {
    strategy: SearchStrategy,
    indexing_mode: IndexingMode,
}

impl SqliteSearchStrategy {
    /// Creates a new SQLite search strategy with default configuration.
    pub fn new() -> Self {
        Self {
            strategy: SearchStrategy::PrecomputedIndex,
            indexing_mode: IndexingMode::Inline,
        }
    }

    /// Creates with a specific strategy.
    pub fn with_strategy(strategy: SearchStrategy) -> Self {
        Self {
            strategy,
            indexing_mode: IndexingMode::Inline,
        }
    }

    /// Creates with hybrid strategy using specified indexed parameters.
    pub fn hybrid(indexed_params: Vec<String>) -> Self {
        Self {
            strategy: SearchStrategy::Hybrid { indexed_params },
            indexing_mode: IndexingMode::Inline,
        }
    }

    /// Sets the indexing mode.
    pub fn with_indexing_mode(mut self, mode: IndexingMode) -> Self {
        self.indexing_mode = mode;
        self
    }

    /// Returns whether a specific parameter has a pre-computed index.
    pub fn has_precomputed_index(&self, _resource_type: &str, param_name: &str) -> bool {
        match &self.strategy {
            SearchStrategy::PrecomputedIndex => true,
            SearchStrategy::QueryTimeEvaluation => false,
            SearchStrategy::Hybrid { indexed_params } => {
                indexed_params.contains(&param_name.to_string())
            }
        }
    }

    /// Gets the current search strategy.
    pub fn strategy(&self) -> &SearchStrategy {
        &self.strategy
    }
}

impl Default for SqliteSearchStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchStrategyCapability for SqliteSearchStrategy {
    fn supports_precomputed_index(&self) -> bool {
        true
    }

    fn supports_jsonb_evaluation(&self) -> bool {
        // We use PrecomputedIndex strategy exclusively - all searches go through
        // the search_index table. Query-time JSONB evaluation is not implemented.
        false
    }

    fn recommended_strategy(&self) -> SearchStrategy {
        // For SQLite, pre-computed index is the only supported strategy
        SearchStrategy::PrecomputedIndex
    }

    fn jsonb_capabilities(&self) -> JsonbCapabilities {
        // We don't use query-time JSON evaluation - all searches use the
        // pre-computed search_index table. These flags describe resource JSON
        // evaluation; wide `_id` filters use JSON1's json_each separately.
        JsonbCapabilities {
            path_extraction: false,      // Not used - we use search_index
            array_iteration: false,      // Not used - we use search_index
            containment_operator: false, // Not in SQLite
            gin_index: false,            // Not in SQLite
        }
    }

    fn indexing_mode(&self) -> IndexingMode {
        self.indexing_mode.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_strategy() {
        let strategy = SqliteSearchStrategy::new();

        assert!(strategy.supports_precomputed_index());
        assert!(!strategy.supports_jsonb_evaluation()); // We don't use query-time JSONB
        assert!(matches!(
            strategy.recommended_strategy(),
            SearchStrategy::PrecomputedIndex
        ));
    }

    #[test]
    fn test_hybrid_strategy() {
        let strategy =
            SqliteSearchStrategy::hybrid(vec!["identifier".to_string(), "name".to_string()]);

        assert!(strategy.has_precomputed_index("Patient", "identifier"));
        assert!(strategy.has_precomputed_index("Patient", "name"));
        assert!(!strategy.has_precomputed_index("Patient", "birthdate"));
    }

    #[test]
    fn test_jsonb_capabilities() {
        let strategy = SqliteSearchStrategy::new();
        let caps = strategy.jsonb_capabilities();

        // We use PrecomputedIndex exclusively, so JSONB capabilities are not used
        assert!(!caps.path_extraction);
        assert!(!caps.array_iteration);
        assert!(!caps.containment_operator);
        assert!(!caps.gin_index);
    }

    #[test]
    fn test_precomputed_index_strategy() {
        let strategy = SqliteSearchStrategy::with_strategy(SearchStrategy::PrecomputedIndex);

        assert!(strategy.has_precomputed_index("Patient", "identifier"));
        assert!(strategy.has_precomputed_index("Patient", "anything"));
    }

    #[test]
    fn test_query_time_strategy() {
        let strategy = SqliteSearchStrategy::with_strategy(SearchStrategy::QueryTimeEvaluation);

        assert!(!strategy.has_precomputed_index("Patient", "identifier"));
        assert!(!strategy.has_precomputed_index("Patient", "anything"));
    }
}
