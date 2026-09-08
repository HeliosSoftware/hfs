// Shared scripted paging source for the real private consumer boundaries.
use crate::core::bulk_submit::{
    BulkEntryResult, EntryResultContinuation, EntryResultPage, PagedEntryResult, entry_result_pages,
};
use crate::error::StorageResult;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub fn pages() -> (
    impl futures::Stream<Item = StorageResult<EntryResultPage>>,
    Arc<AtomicUsize>,
) {
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let script = [
        (
            None,
            EntryResultPage {
                entries: Vec::new(),
                next: Some(EntryResultContinuation::Offset(17)),
            },
        ),
        (
            Some(EntryResultContinuation::Offset(17)),
            EntryResultPage {
                entries: ["after-empty", "after-empty", "exclusive-late"]
                    .into_iter()
                    .enumerate()
                    .map(|(line, id)| PagedEntryResult {
                        result: BulkEntryResult::success(line as u64, "Patient", id, true),
                        stored_identity: None,
                    })
                    .collect(),
                next: Some(EntryResultContinuation::Offset(91)),
            },
        ),
        (
            Some(EntryResultContinuation::Offset(91)),
            EntryResultPage {
                entries: Vec::new(),
                next: None,
            },
        ),
    ];
    let mut script = std::collections::VecDeque::from(script);
    let pages = entry_result_pages(move |continuation| {
        let (expected, page) = script.pop_front().expect("must not fetch after EOF");
        assert_eq!(
            continuation, expected,
            "consumer passes the opaque token unchanged"
        );
        observed.fetch_add(1, Ordering::SeqCst);
        std::future::ready(Ok(page))
    });
    (pages, calls)
}
