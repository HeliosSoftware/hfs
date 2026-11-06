//! Splitting one resource type into id ranges for a multi-stream `$reindex`
//! ([`ReindexSource::id_range_boundaries`](super::ReindexSource::id_range_boundaries)),
//! shared by the SQLite, PostgreSQL and MongoDB sources. Ids compare byte by
//! byte in every source that supports ranges (#1739, #1767), so the same
//! boundaries mean the same ranges everywhere.
//!
//! Two ways to pick the boundaries, chosen per type from a sample of its
//! ids: when the ids are canonical UUIDs — every FHIR server's default, and
//! what every bulk-generated corpus carries — the hex space is cut into
//! equal slices without reading the type at all; otherwise the type is
//! counted and the ids at the `i × total / streams` positions of its id
//! order are read, one bounded index walk each. Either way any boundaries
//! are correct; only the balance of the streams is at stake.

use std::future::Future;

use crate::error::StorageResult;

/// Ids sampled from each end of a type's id order to decide its split.
pub(crate) const SPLIT_SAMPLE: u64 = 32;

/// Whether `id` is a canonical lowercase UUID (`8-4-4-4-12` hex digits).
pub(crate) fn is_canonical_uuid(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => *b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(b),
        })
}

/// Whether every sampled id is a canonical UUID: the type's ids are then
/// spread evenly over the hex space and can be split without counting.
pub(crate) fn sample_is_uuid(sample: &[String]) -> bool {
    !sample.is_empty() && sample.iter().all(|id| is_canonical_uuid(id))
}

/// Boundaries that cut the UUID space into `streams` equal slices: the two
/// leading hex digits of `i × 256 / streams` for `i` in `1..streams`. Two
/// digits make every boundary a prefix a byte-ordered index seeks on;
/// consecutive duplicates (only past 256 streams) are dropped.
pub(crate) fn uuid_hex_boundaries(streams: usize) -> Vec<String> {
    let streams = streams.max(1);
    let mut boundaries: Vec<String> = Vec::with_capacity(streams - 1);
    for i in 1..streams {
        let boundary = format!("{:02x}", i * 256 / streams);
        if boundaries.last() != Some(&boundary) {
            boundaries.push(boundary);
        }
    }
    boundaries
}

/// The 0-based positions, in id order, of the ids that split `total`
/// resources into `streams` ranges of nearly equal size: the id at
/// `i × total / streams` starts range `i`. Distinct and ascending.
pub(crate) fn split_positions(total: u64, streams: usize) -> Vec<u64> {
    let streams = streams.max(1) as u64;
    let mut positions: Vec<u64> = (1..streams)
        .map(|i| (u128::from(i) * u128::from(total) / u128::from(streams)) as u64)
        .filter(|position| *position > 0)
        .collect();
    positions.dedup();
    positions
}

/// The shared decision behind every source's `id_range_boundaries`.
///
/// `ids_at(offset, limit, descending)` reads `limit` ids of the type from
/// position `offset` of its id order (ascending, or descending for a sample
/// from the top); `count()` is the type's live resource count, read only
/// when the ids are not UUIDs. A type of fewer than
/// `streams × min_per_stream` resources is not split (an empty vector), which
/// the first sample settles without a count.
pub(crate) async fn split_boundaries<F, Fut, C, CFut>(
    streams: usize,
    min_per_stream: u64,
    ids_at: F,
    count: C,
) -> StorageResult<Vec<String>>
where
    F: Fn(u64, u64, bool) -> Fut,
    Fut: Future<Output = StorageResult<Vec<String>>>,
    C: FnOnce() -> CFut,
    CFut: Future<Output = StorageResult<u64>>,
{
    if streams < 2 {
        return Ok(Vec::new());
    }
    let threshold = (streams as u64).saturating_mul(min_per_stream).max(1);
    // The ids just below the threshold position: fewer than asked means
    // the type is too small to split, and no count was needed to know.
    let need = threshold.min(SPLIT_SAMPLE);
    let head = ids_at(threshold - need, need, false).await?;
    if (head.len() as u64) < need {
        return Ok(Vec::new());
    }
    let tail = ids_at(0, SPLIT_SAMPLE, true).await?;
    if sample_is_uuid(&head) && sample_is_uuid(&tail) {
        return Ok(uuid_hex_boundaries(streams));
    }
    let total = count().await?;
    if total < threshold {
        return Ok(Vec::new());
    }
    let mut boundaries = Vec::with_capacity(streams - 1);
    for position in split_positions(total, streams) {
        if let Some(id) = ids_at(position, 1, false).await?.into_iter().next() {
            boundaries.push(id);
        }
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    Ok(boundaries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn canonical_uuids_are_recognised_and_nothing_else_is() {
        assert!(is_canonical_uuid("5e2a0b3c-9d1f-4a8e-b7c6-0123456789ab"));
        assert!(!is_canonical_uuid("5E2A0B3C-9D1F-4A8E-B7C6-0123456789AB"));
        assert!(!is_canonical_uuid("5e2a0b3c9d1f4a8eb7c60123456789ab"));
        assert!(!is_canonical_uuid("patient-1"));
        assert!(!is_canonical_uuid(""));
        assert!(!is_canonical_uuid("5e2a0b3c-9d1f-4a8e-b7c6-0123456789ag"));
        assert!(!sample_is_uuid(&[]));
    }

    #[test]
    fn hex_boundaries_cut_the_uuid_space_evenly() {
        assert_eq!(uuid_hex_boundaries(1), Vec::<String>::new());
        assert_eq!(uuid_hex_boundaries(2), ["80"]);
        assert_eq!(uuid_hex_boundaries(3), ["55", "aa"]);
        assert_eq!(
            uuid_hex_boundaries(8),
            ["20", "40", "60", "80", "a0", "c0", "e0"]
        );
        // Every boundary orders a UUID with that prefix into the range it starts.
        let boundaries = uuid_hex_boundaries(4);
        assert!("40000000-0000-4000-8000-000000000000" >= boundaries[0].as_str());
        assert!("3fffffff-ffff-4fff-bfff-ffffffffffff" < boundaries[0].as_str());
    }

    #[test]
    fn split_positions_are_equal_shares_of_the_total() {
        assert_eq!(split_positions(100, 4), [25, 50, 75]);
        assert_eq!(split_positions(10, 3), [3, 6]);
        assert_eq!(split_positions(2, 4), [1]);
        assert_eq!(split_positions(0, 4), Vec::<u64>::new());
        assert_eq!(split_positions(100, 1), Vec::<u64>::new());
    }

    /// A type of `ids` (in id order), read through the two closures the
    /// decision takes; records every read.
    async fn boundaries_of(
        ids: &[&str],
        streams: usize,
        min_per_stream: u64,
    ) -> (Vec<String>, Vec<(u64, u64, bool)>, bool) {
        let reads = RefCell::new(Vec::new());
        let counted = RefCell::new(false);
        let boundaries = split_boundaries(
            streams,
            min_per_stream,
            |offset, limit, descending| {
                reads.borrow_mut().push((offset, limit, descending));
                let mut ordered: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
                if descending {
                    ordered.reverse();
                }
                let out: Vec<String> = ordered
                    .into_iter()
                    .skip(offset as usize)
                    .take(limit as usize)
                    .collect();
                async move { Ok(out) }
            },
            || {
                *counted.borrow_mut() = true;
                let total = ids.len() as u64;
                async move { Ok(total) }
            },
        )
        .await
        .expect("split");
        (boundaries, reads.into_inner(), counted.into_inner())
    }

    fn uuids(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| format!("{:08x}-0000-4000-8000-000000000000", i * 0x0100_0000))
            .collect()
    }

    #[tokio::test]
    async fn one_stream_never_reads() {
        let ids = uuids(10);
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let (boundaries, reads, counted) = boundaries_of(&refs, 1, 1).await;
        assert!(boundaries.is_empty());
        assert!(reads.is_empty());
        assert!(!counted);
    }

    #[tokio::test]
    async fn a_type_below_the_threshold_is_not_split_and_not_counted() {
        let ids = uuids(10);
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let (boundaries, reads, counted) = boundaries_of(&refs, 4, 3).await;
        assert!(boundaries.is_empty());
        assert_eq!(reads, [(0, 12, false)], "one bounded read settles it");
        assert!(!counted);
    }

    #[tokio::test]
    async fn uuid_ids_are_split_on_the_hex_space_without_a_count() {
        let ids = uuids(200);
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let (boundaries, reads, counted) = boundaries_of(&refs, 4, 10).await;
        assert_eq!(boundaries, ["40", "80", "c0"]);
        assert_eq!(reads, [(8, 32, false), (0, 32, true)]);
        assert!(!counted);
    }

    #[tokio::test]
    async fn other_ids_are_split_at_equal_positions_after_a_count() {
        let ids: Vec<String> = (0..200).map(|i| format!("p{i:04}")).collect();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let (boundaries, reads, counted) = boundaries_of(&refs, 4, 10).await;
        assert_eq!(boundaries, ["p0050", "p0100", "p0150"]);
        assert!(counted);
        assert_eq!(
            &reads[2..],
            [(50, 1, false), (100, 1, false), (150, 1, false)]
        );
    }

    #[tokio::test]
    async fn a_mixed_sample_takes_the_counted_path() {
        let mut ids = uuids(100);
        ids.extend((0..100).map(|i| format!("p{i:04}")));
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let (boundaries, _, counted) = boundaries_of(&refs, 2, 10).await;
        assert!(counted);
        assert_eq!(boundaries, ["p0000"]);
    }
}
