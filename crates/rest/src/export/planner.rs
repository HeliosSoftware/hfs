//! Shard planner for `$viewdefinition-export`.
//!
//! Given a total row count and a target shard size, [`plan`] returns the
//! row-index ranges that each shard should cover.  The caller is responsible
//! for slicing the materialised row `Vec` accordingly and writing each slice to
//! its own output file.
//!
//! ## Example
//!
//! ```
//! use helios_rest::export::planner::plan;
//!
//! // 1100 rows at 500 rows / shard → 3 shards: [0,500), [500,1000), [1000,1100)
//! let shards = plan(1100, 500);
//! assert_eq!(shards.len(), 3);
//! assert_eq!(shards[0], 0..500);
//! assert_eq!(shards[1], 500..1000);
//! assert_eq!(shards[2], 1000..1100);
//! ```

use std::ops::Range;

/// Target rows per output shard when `HFS_EXPORT_SHARD_ROWS` is not set.
pub const DEFAULT_SHARD_ROWS: usize = 500_000;

/// Splits `total_rows` into contiguous [`Range`]s each of size at most
/// `shard_size`.
///
/// Returns an empty `Vec` when `total_rows == 0`.
/// Returns a single `0..total_rows` range when `shard_size == 0` (treat as
/// unlimited — caller must guard against this).
pub fn plan(total_rows: usize, shard_size: usize) -> Vec<Range<usize>> {
    if total_rows == 0 {
        return Vec::new();
    }
    // Treat zero / very large shard_size as "one shard containing everything".
    let effective_size = if shard_size == 0 {
        total_rows
    } else {
        shard_size
    };

    let num_shards = total_rows.div_ceil(effective_size);
    (0..num_shards)
        .map(|i| {
            let start = i * effective_size;
            let end = ((i + 1) * effective_size).min(total_rows);
            start..end
        })
        .collect()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_empty() {
        assert!(plan(0, 500).is_empty());
    }

    #[test]
    fn test_plan_single_shard_exact() {
        let shards = plan(500, 500);
        assert_eq!(shards, vec![0..500]);
    }

    #[test]
    fn test_plan_single_shard_under() {
        let shards = plan(100, 500);
        assert_eq!(shards, vec![0..100]);
    }

    #[test]
    fn test_plan_multi_shard_exact() {
        let shards = plan(1000, 500);
        assert_eq!(shards, vec![0..500, 500..1000]);
    }

    #[test]
    fn test_plan_multi_shard_with_remainder() {
        let shards = plan(1100, 500);
        assert_eq!(shards, vec![0..500, 500..1000, 1000..1100]);
    }

    #[test]
    fn test_plan_zero_shard_size_gives_one_shard() {
        let shards = plan(300, 0);
        assert_eq!(shards, vec![0..300]);
    }
}
