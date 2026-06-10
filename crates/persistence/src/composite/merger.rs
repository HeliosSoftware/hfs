//! Result merging strategies for composite storage.
//!
//! This module provides strategies for merging search results from multiple backends.
//!
//! # Merge Strategies
//!
//! | Strategy | Description | Use Case |
//! |----------|-------------|----------|
//! | Intersection | Results must appear in all sources | Precise filtering |
//! | Union | Results from any source | Broad search |
//! | PrimaryEnriched | Primary results, enriched by secondaries | Metadata augmentation |
//! | SecondaryFiltered | Filter secondary results through primary | Candidate validation |
//!
//! # Example
//!
//! ```ignore
//! use helios_persistence::composite::merger::{ResultMerger, MergeOptions, MergeStrategy};
//!
//! let merger = ResultMerger::new();
//!
//! let merged = merger.merge(
//!     primary_result,
//!     vec![("es".to_string(), es_result)],
//!     MergeOptions {
//!         strategy: MergeStrategy::Intersection,
//!         preserve_primary_order: true,
//!         deduplicate: true,
//!     },
//! )?;
//! ```

use std::collections::{HashMap, HashSet};

use crate::core::SearchResult;
use crate::error::StorageResult;
use crate::types::{Page, PageInfo, StoredResource};

use super::router::MergeStrategy;

/// Options for merging results.
#[derive(Debug, Clone)]
pub struct MergeOptions {
    /// The merge strategy to use.
    pub strategy: MergeStrategy,

    /// Whether to preserve the ordering from the primary result.
    pub preserve_primary_order: bool,

    /// Whether to deduplicate results.
    pub deduplicate: bool,
}

impl Default for MergeOptions {
    fn default() -> Self {
        Self {
            strategy: MergeStrategy::Intersection,
            preserve_primary_order: true,
            deduplicate: true,
        }
    }
}

/// Result merger for combining results from multiple backends.
pub struct ResultMerger {
    /// Maximum results to return.
    max_results: usize,
}

impl ResultMerger {
    /// Creates a new result merger.
    pub fn new() -> Self {
        Self { max_results: 1000 }
    }

    /// Creates a merger with a custom max results limit.
    pub fn with_max_results(mut self, max: usize) -> Self {
        self.max_results = max;
        self
    }

    /// Merges results from primary and auxiliary backends.
    pub fn merge(
        &self,
        primary: SearchResult,
        auxiliary: Vec<(String, SearchResult)>,
        options: MergeOptions,
    ) -> StorageResult<SearchResult> {
        match options.strategy {
            MergeStrategy::Intersection => self.merge_intersection(primary, auxiliary, &options),
            MergeStrategy::Union => self.merge_union(primary, auxiliary, &options),
            MergeStrategy::PrimaryEnriched => {
                self.merge_primary_enriched(primary, auxiliary, &options)
            }
            MergeStrategy::SecondaryFiltered => {
                self.merge_secondary_filtered(primary, auxiliary, &options)
            }
        }
    }

    /// Intersection merge: results must appear in all sources.
    fn merge_intersection(
        &self,
        primary: SearchResult,
        auxiliary: Vec<(String, SearchResult)>,
        options: &MergeOptions,
    ) -> StorageResult<SearchResult> {
        if auxiliary.is_empty() {
            return Ok(primary);
        }

        // Build set of IDs from each auxiliary source
        let aux_id_sets: Vec<HashSet<String>> = auxiliary
            .iter()
            .map(|(_, result)| result.resources.items.iter().map(resource_key).collect())
            .collect();

        // Filter primary to only include resources that appear in ALL auxiliary sets
        let mut filtered_items = Vec::new();
        for resource in primary.resources.items {
            let key = resource_key(&resource);
            if aux_id_sets.iter().all(|set| set.contains(&key)) {
                filtered_items.push(resource);
            }
        }

        // Limit results
        if filtered_items.len() > self.max_results {
            filtered_items.truncate(self.max_results);
        }

        // Combine included resources
        let mut all_included = primary.included;
        for (_, aux_result) in auxiliary {
            all_included.extend(aux_result.included);
        }

        if options.deduplicate {
            all_included = deduplicate_resources(all_included);
        }

        Ok(SearchResult {
            resources: Page::new(filtered_items, primary.resources.page_info),
            included: all_included,
            total: None, // Total is now uncertain due to filtering
            scores: primary.scores,
        })
    }

    /// Union merge: results from any source (OR).
    fn merge_union(
        &self,
        primary: SearchResult,
        auxiliary: Vec<(String, SearchResult)>,
        options: &MergeOptions,
    ) -> StorageResult<SearchResult> {
        let mut all_resources = primary.resources.items;
        let mut seen_keys: HashSet<String> = all_resources.iter().map(resource_key).collect();

        // Add resources from auxiliary sources
        for (_, aux_result) in auxiliary {
            for resource in aux_result.resources.items {
                let key = resource_key(&resource);
                if !seen_keys.contains(&key) {
                    seen_keys.insert(key);
                    all_resources.push(resource);
                }
            }
        }

        // Sort if not preserving primary order
        if !options.preserve_primary_order {
            // Sort by last updated, descending
            all_resources.sort_by_key(|r| std::cmp::Reverse(r.last_modified()));
        }

        // Limit results
        if all_resources.len() > self.max_results {
            all_resources.truncate(self.max_results);
        }

        Ok(SearchResult {
            resources: Page::new(all_resources, primary.resources.page_info),
            included: primary.included,
            total: None,
            scores: primary.scores,
        })
    }

    /// Primary enriched: primary results with metadata from secondaries.
    fn merge_primary_enriched(
        &self,
        primary: SearchResult,
        _auxiliary: Vec<(String, SearchResult)>,
        _options: &MergeOptions,
    ) -> StorageResult<SearchResult> {
        // For FHIR resources, we generally don't want to modify the content
        // This strategy is more about adding metadata that doesn't change resources
        // For now, just return primary results unchanged
        Ok(primary)
    }

    /// Secondary filtered: filter secondary results through primary.
    fn merge_secondary_filtered(
        &self,
        primary: SearchResult,
        auxiliary: Vec<(String, SearchResult)>,
        _options: &MergeOptions,
    ) -> StorageResult<SearchResult> {
        if auxiliary.is_empty() {
            return Ok(primary);
        }

        // Get IDs from all auxiliary sources (union of auxiliary IDs)
        let mut aux_ids: HashSet<String> = HashSet::new();
        for (_, aux_result) in &auxiliary {
            for resource in &aux_result.resources.items {
                aux_ids.insert(resource_key(resource));
            }
        }

        // Filter primary to only include resources that appear in auxiliary results
        let filtered_items: Vec<_> = primary
            .resources
            .items
            .into_iter()
            .filter(|r| aux_ids.contains(&resource_key(r)))
            .take(self.max_results)
            .collect();

        Ok(SearchResult {
            resources: Page::new(filtered_items, primary.resources.page_info),
            included: primary.included,
            total: None,
            scores: primary.scores,
        })
    }

    /// Merges ID sets from multiple sources.
    pub fn merge_ids(&self, sources: Vec<Vec<String>>, strategy: MergeStrategy) -> Vec<String> {
        match strategy {
            MergeStrategy::Intersection => self.intersect_ids(sources),
            MergeStrategy::Union => self.union_ids(sources),
            _ => self.intersect_ids(sources),
        }
    }

    /// Computes intersection of ID sets.
    fn intersect_ids(&self, sources: Vec<Vec<String>>) -> Vec<String> {
        if sources.is_empty() {
            return Vec::new();
        }

        if sources.len() == 1 {
            return sources.into_iter().next().unwrap();
        }

        let mut sets: Vec<HashSet<String>> = sources
            .into_iter()
            .map(|v| v.into_iter().collect())
            .collect();

        // Sort by size (smallest first for efficiency)
        sets.sort_by_key(|s| s.len());

        let mut result: HashSet<String> = sets.remove(0);
        for set in sets {
            result = result.intersection(&set).cloned().collect();
        }

        result.into_iter().collect()
    }

    /// Computes union of ID sets.
    fn union_ids(&self, sources: Vec<Vec<String>>) -> Vec<String> {
        let mut result: HashSet<String> = HashSet::new();
        for source in sources {
            result.extend(source);
        }
        result.into_iter().collect()
    }
}

impl Default for ResultMerger {
    fn default() -> Self {
        Self::new()
    }
}

/// Creates a unique key for a resource.
fn resource_key(resource: &StoredResource) -> String {
    format!("{}/{}", resource.resource_type(), resource.id())
}

/// Deduplicates resources by their key.
fn deduplicate_resources(resources: Vec<StoredResource>) -> Vec<StoredResource> {
    let mut seen = HashSet::new();
    resources
        .into_iter()
        .filter(|r| seen.insert(resource_key(r)))
        .collect()
}

/// Weighted result for relevance-based merging.
#[derive(Debug, Clone)]
pub struct WeightedResult {
    /// The resource.
    pub resource: StoredResource,

    /// Relevance score (higher is better).
    pub score: f64,

    /// Source backend ID.
    pub source: String,
}

/// Relevance-based merger for search results.
pub struct RelevanceMerger {
    /// Backend weights for scoring.
    weights: HashMap<String, f64>,
}

impl RelevanceMerger {
    /// Creates a new relevance merger.
    pub fn new() -> Self {
        Self {
            weights: HashMap::new(),
        }
    }

    /// Sets weight for a backend.
    pub fn with_weight(mut self, backend_id: impl Into<String>, weight: f64) -> Self {
        self.weights.insert(backend_id.into(), weight);
        self
    }

    /// Merges results with relevance scoring.
    pub fn merge_with_relevance(
        &self,
        results: Vec<(String, SearchResult)>,
        max_results: usize,
    ) -> SearchResult {
        let mut weighted: Vec<WeightedResult> = Vec::new();

        for (source, result) in results {
            let base_weight = self.weights.get(&source).copied().unwrap_or(1.0);

            for (idx, resource) in result.resources.items.into_iter().enumerate() {
                // Score based on position and source weight
                // Earlier positions get higher scores
                let position_score = 1.0 / (idx as f64 + 1.0);
                let score = position_score * base_weight;

                weighted.push(WeightedResult {
                    resource,
                    score,
                    source: source.clone(),
                });
            }
        }

        // Sort by score descending
        weighted.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Deduplicate, keeping highest scored
        let mut seen = HashSet::new();
        let final_results: Vec<StoredResource> = weighted
            .into_iter()
            .filter(|w| seen.insert(resource_key(&w.resource)))
            .take(max_results)
            .map(|w| w.resource)
            .collect();

        SearchResult {
            resources: Page::new(final_results, PageInfo::end()),
            included: Vec::new(),
            total: None,
            scores: Default::default(),
        }
    }
}

impl Default for RelevanceMerger {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tenant::TenantId;
    use helios_fhir::FhirVersion;

    fn make_resource(resource_type: &str, id: &str) -> StoredResource {
        StoredResource::new(
            resource_type,
            id,
            TenantId::new("test"),
            serde_json::json!({"resourceType": resource_type, "id": id}),
            FhirVersion::default(),
        )
    }

    fn make_result(resources: Vec<StoredResource>) -> SearchResult {
        SearchResult {
            resources: Page::new(resources, PageInfo::end()),
            included: Vec::new(),
            total: None,
            scores: Default::default(),
        }
    }

    #[test]
    fn test_intersection_merge() {
        let merger = ResultMerger::new();

        let primary = make_result(vec![
            make_resource("Patient", "1"),
            make_resource("Patient", "2"),
            make_resource("Patient", "3"),
        ]);

        let aux = vec![(
            "es".to_string(),
            make_result(vec![
                make_resource("Patient", "2"),
                make_resource("Patient", "3"),
                make_resource("Patient", "4"),
            ]),
        )];

        let merged = merger.merge(primary, aux, MergeOptions::default()).unwrap();

        // Only 2 and 3 should remain (intersection)
        assert_eq!(merged.resources.len(), 2);
        let ids: Vec<_> = merged.resources.items.iter().map(|r| r.id()).collect();
        assert!(ids.contains(&"2"));
        assert!(ids.contains(&"3"));
    }

    #[test]
    fn test_union_merge() {
        let merger = ResultMerger::new();

        let primary = make_result(vec![
            make_resource("Patient", "1"),
            make_resource("Patient", "2"),
        ]);

        let aux = vec![(
            "es".to_string(),
            make_result(vec![
                make_resource("Patient", "2"),
                make_resource("Patient", "3"),
            ]),
        )];

        let merged = merger
            .merge(
                primary,
                aux,
                MergeOptions {
                    strategy: MergeStrategy::Union,
                    ..Default::default()
                },
            )
            .unwrap();

        // All unique resources (1, 2, 3)
        assert_eq!(merged.resources.len(), 3);
    }

    #[test]
    fn test_secondary_filtered_merge() {
        let merger = ResultMerger::new();

        let primary = make_result(vec![
            make_resource("Patient", "1"),
            make_resource("Patient", "2"),
            make_resource("Patient", "3"),
        ]);

        let aux = vec![(
            "graph".to_string(),
            make_result(vec![make_resource("Patient", "2")]),
        )];

        let merged = merger
            .merge(
                primary,
                aux,
                MergeOptions {
                    strategy: MergeStrategy::SecondaryFiltered,
                    ..Default::default()
                },
            )
            .unwrap();

        // Only 2 should remain (filtered by secondary)
        assert_eq!(merged.resources.len(), 1);
        assert_eq!(merged.resources.items[0].id(), "2");
    }

    #[test]
    fn test_id_intersection() {
        let merger = ResultMerger::new();

        let sources = vec![
            vec!["1".to_string(), "2".to_string(), "3".to_string()],
            vec!["2".to_string(), "3".to_string(), "4".to_string()],
            vec!["3".to_string(), "4".to_string(), "5".to_string()],
        ];

        let result = merger.merge_ids(sources, MergeStrategy::Intersection);
        assert_eq!(result.len(), 1);
        assert!(result.contains(&"3".to_string()));
    }

    #[test]
    fn test_id_union() {
        let merger = ResultMerger::new();

        let sources = vec![
            vec!["1".to_string(), "2".to_string()],
            vec!["3".to_string(), "4".to_string()],
        ];

        let result = merger.merge_ids(sources, MergeStrategy::Union);
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn test_relevance_merge() {
        let merger = RelevanceMerger::new()
            .with_weight("primary", 2.0)
            .with_weight("search", 1.0);

        let results = vec![
            (
                "primary".to_string(),
                make_result(vec![
                    make_resource("Patient", "1"),
                    make_resource("Patient", "2"),
                ]),
            ),
            (
                "search".to_string(),
                make_result(vec![
                    make_resource("Patient", "3"),
                    make_resource("Patient", "1"), // duplicate
                ]),
            ),
        ];

        let merged = merger.merge_with_relevance(results, 10);

        // Patient 1 should be first (highest weight from primary)
        assert_eq!(merged.resources.items[0].id(), "1");
        // Should have 3 unique resources
        assert_eq!(merged.resources.len(), 3);
    }
}
