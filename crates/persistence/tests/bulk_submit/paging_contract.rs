// Included by the SQLite unit tests and PostgreSQL integration tests.
// The including module supplies `persistence` as an alias for the library.
use persistence::core::bulk_submit::{
    BulkEntryOutcome, BulkSubmitProvider, EntryResultContinuation, EntryResultCursor,
    PagedEntryResult, SubmissionId,
};
use persistence::tenant::{TenantContext, TenantId, TenantPermissions};

pub struct ReceiptRow {
    pub file: String,
    pub line: i64,
    pub id: String,
    pub outcome: &'static str,
}

#[async_trait::async_trait]
pub trait ReceiptFixture: BulkSubmitProvider {
    /// Insert the supplied scope verbatim, including an identical manifest id
    /// in different scopes, so each scope predicate is independently tested.
    async fn seed_receipts(
        &self,
        tenant: &TenantContext,
        submission: &SubmissionId,
        manifest: &str,
        rows: &[ReceiptRow],
    );
}

async fn collect_pages<B: BulkSubmitProvider>(
    backend: &B,
    tenant: &TenantContext,
    submission: &SubmissionId,
    manifest: &str,
    filter: Option<BulkEntryOutcome>,
    limit: u32,
) -> Vec<PagedEntryResult> {
    let mut next = None;
    let mut all = Vec::new();
    // A bound makes a repeated token fail instead of hanging a test forever.
    for _ in 0..100 {
        let page = backend
            .get_entry_results_page(tenant, submission, manifest, filter, limit, next.as_ref())
            .await
            .unwrap();
        assert!(page.entries.len() <= limit as usize);
        if let Some(token) = &page.next {
            let EntryResultContinuation::Keyset(cursor) = token else {
                panic!("SQL returned an OFFSET continuation");
            };
            assert_eq!(
                Some(cursor),
                page.entries.last().unwrap().stored_identity.as_ref()
            );
            assert_ne!(Some(token), next.as_ref(), "continuation must advance");
        }
        for entry in &page.entries {
            assert_eq!(
                entry.stored_identity.as_ref().unwrap().line_number,
                entry.result.line_number
            );
        }
        all.extend(page.entries);
        next = page.next;
        if next.is_none() {
            return all;
        }
    }
    panic!("receipt traversal did not terminate");
}

pub async fn exact_sql_pages<B: ReceiptFixture>(
    backend: &B,
    tenant: &TenantContext,
    max_line: i64,
) {
    let submission = SubmissionId::generate("receipt-paging");
    let manifest = "same-manifest";
    let rows: Vec<_> = ["", "a.ndjson", "b.ndjson", "c.ndjson", "d.ndjson"]
        .into_iter()
        .flat_map(|file| {
            (0..7).map(move |line| ReceiptRow {
                file: file.to_string(),
                line,
                id: format!("repeated-{}", line % 2),
                outcome: match line {
                    0 | 6 => "success",
                    1 | 2 => "skipped",
                    3 | 4 => "validation-error",
                    _ => "processing-error",
                },
            })
        })
        .collect();
    backend
        .seed_receipts(tenant, &submission, manifest, &rows)
        .await;

    // Only one scope component differs in each distractor. Reusing identical
    // stored identities exposes a missing predicate even when counts look right.
    let other_tenant = TenantContext::new(
        TenantId::new(format!("{}-other", tenant.tenant_id().as_str())),
        TenantPermissions::full_access(),
    );
    let other_submitter = SubmissionId::new("other-submitter", &submission.submission_id);
    let other_submission = SubmissionId::new(&submission.submitter, "other-submission");
    let distractors = [ReceiptRow {
        file: "a.ndjson".to_string(),
        line: 0,
        id: "scope-leak".to_string(),
        outcome: "success",
    }];
    for (scope_tenant, scope_submission, scope_manifest) in [
        (&other_tenant, &submission, manifest),
        (tenant, &other_submitter, manifest),
        (tenant, &other_submission, manifest),
        (tenant, &submission, "other-manifest"),
    ] {
        backend
            .seed_receipts(scope_tenant, scope_submission, scope_manifest, &distractors)
            .await;
    }

    for filter in [
        None,
        Some(BulkEntryOutcome::Success),
        Some(BulkEntryOutcome::Skipped),
        Some(BulkEntryOutcome::ValidationError),
        Some(BulkEntryOutcome::ProcessingError),
    ] {
        let expected: Vec<_> = rows
            .iter()
            .filter(|row| filter.is_none_or(|f| f.to_string() == row.outcome))
            .collect();
        // One row, tied boundaries, exact multiples, >2 pages, and a single page.
        for limit in [1, 2, 5, 7, 35, 100] {
            let actual = collect_pages(backend, tenant, &submission, manifest, filter, limit).await;
            let identities: Vec<_> = actual
                .iter()
                .map(|entry| {
                    let identity = entry.stored_identity.as_ref().unwrap();
                    (identity.file_url.as_str(), identity.line_number)
                })
                .collect();
            assert_eq!(
                identities,
                expected
                    .iter()
                    .map(|row| (row.file.as_str(), row.line as u64))
                    .collect::<Vec<_>>()
            );
            let mut references: Vec<_> = actual
                .iter()
                .map(|entry| {
                    assert!(
                        !entry.result.created,
                        "NULL created retains its existing meaning"
                    );
                    format!(
                        "{}/{}",
                        entry.result.resource_type,
                        entry.result.resource_id.as_ref().unwrap()
                    )
                })
                .collect();
            references.sort();
            let mut expected_refs: Vec<_> = expected
                .iter()
                .map(|row| format!("Patient/{}", row.id))
                .collect();
            expected_refs.sort();
            assert_eq!(
                references, expected_refs,
                "receipt multiplicity must survive pagination"
            );
        }
    }

    let empty = collect_pages(backend, tenant, &submission, "empty-manifest", None, 5).await;
    assert!(empty.is_empty());
    assert!(
        backend
            .get_entry_results_page(tenant, &submission, manifest, None, 0, None)
            .await
            .is_err()
    );
    assert!(
        backend
            .get_entry_results_page(
                tenant,
                &submission,
                manifest,
                None,
                1,
                Some(&EntryResultContinuation::Offset(0))
            )
            .await
            .is_err()
    );
    assert!(
        backend
            .get_entry_results_page(
                tenant,
                &submission,
                manifest,
                None,
                1,
                Some(&EntryResultContinuation::Keyset(EntryResultCursor {
                    file_url: String::new(),
                    line_number: u64::MAX
                }))
            )
            .await
            .is_err()
    );

    backend
        .seed_receipts(
            tenant,
            &submission,
            "maximum-line",
            &[ReceiptRow {
                file: String::new(),
                line: max_line,
                id: "maximum".to_string(),
                outcome: "success",
            }],
        )
        .await;
    let maximum = collect_pages(backend, tenant, &submission, "maximum-line", None, 1).await;
    assert_eq!(
        maximum[0].stored_identity.as_ref().unwrap().line_number,
        max_line as u64
    );
    assert!(
        backend
            .get_entry_results_page(
                tenant,
                &submission,
                manifest,
                None,
                1,
                Some(&EntryResultContinuation::Keyset(EntryResultCursor {
                    file_url: String::new(),
                    line_number: max_line as u64 + 1
                }))
            )
            .await
            .is_err()
    );

    backend
        .seed_receipts(
            tenant,
            &submission,
            "negative-line",
            &[ReceiptRow {
                file: "bad.ndjson".to_string(),
                line: -1,
                id: "negative".to_string(),
                outcome: "success",
            }],
        )
        .await;
    assert!(
        backend
            .get_entry_results_page(tenant, &submission, "negative-line", None, 10, None)
            .await
            .is_err()
    );
}
