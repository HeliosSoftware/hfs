//! Backend-agnostic contract for [`SecondarySyncFailureLedger`] (#1334): the
//! durable "needs reindex" records a composite keeps in its primary.
//!
//! The ledger is shared by every test that runs against the same database,
//! so everything here is scoped to a tenant id unique to the caller and the
//! global count is only ever compared with itself.

use chrono::{Duration, Utc};
use helios_persistence::composite::{
    SecondarySyncFailure, SecondarySyncFailureLedger, SyncFailureKey, SyncFailureReport,
    SyncOperation,
};

fn key(tenant: &str, id: &str, backend: &str) -> SyncFailureKey {
    SyncFailureKey {
        tenant_id: tenant.to_string(),
        resource_type: "Patient".to_string(),
        resource_id: id.to_string(),
        backend_id: backend.to_string(),
    }
}

async fn mine(ledger: &dyn SecondarySyncFailureLedger, tenant: &str) -> Vec<SecondarySyncFailure> {
    ledger
        .list_sync_failures(100_000)
        .await
        .expect("list")
        .into_iter()
        .filter(|record| record.key.tenant_id == tenant)
        .collect()
}

/// Records fold per (tenant, type, id, backend), list least-recently-failed
/// first, and clear exactly once.
pub async fn ledger_folds_orders_clears_and_counts(
    ledger: &dyn SecondarySyncFailureLedger,
    tenant: &str,
) {
    assert!(
        mine(ledger, tenant).await.is_empty(),
        "control: clean slate"
    );
    let before = ledger.count_sync_failures().await.expect("count");

    let t0 = Utc::now() - Duration::seconds(30);
    let report = |id: &str, backend: &str, operation, seconds, error: &str| SyncFailureReport {
        key: key(tenant, id, backend),
        operation,
        attempts: 4,
        error: error.to_string(),
        failed_at: t0 + Duration::seconds(seconds),
    };

    let first = report("a", "es", SyncOperation::Create, 0, "first");
    assert!(ledger.record_sync_failure(&first).await.expect("record"));
    assert!(
        ledger
            .record_sync_failure(&report("b", "es", SyncOperation::Update, 1, "b"))
            .await
            .expect("record")
    );
    // Same resource, another secondary: its own record.
    assert!(
        ledger
            .record_sync_failure(&report("a", "graph", SyncOperation::Create, 2, "g"))
            .await
            .expect("record")
    );
    // Same resource, same secondary: folded, not inserted.
    assert!(
        !ledger
            .record_sync_failure(&report("a", "es", SyncOperation::Delete, 3, "latest"))
            .await
            .expect("record")
    );

    let records = mine(ledger, tenant).await;
    let order: Vec<_> = records
        .iter()
        .map(|r| (r.key.resource_id.as_str(), r.key.backend_id.as_str()))
        .collect();
    assert_eq!(
        order,
        [("b", "es"), ("a", "graph"), ("a", "es")],
        "least recently failed first; the re-failed record moved to the back"
    );
    let folded = &records[2];
    assert_eq!(folded.operation, SyncOperation::Delete);
    assert_eq!(folded.attempts, 8);
    assert_eq!(folded.last_error, "latest");
    // Stores keep at least millisecond precision.
    let drift =
        |a: chrono::DateTime<Utc>, b: chrono::DateTime<Utc>| (a - b).num_milliseconds().abs();
    assert!(drift(folded.first_failed_at, first.failed_at) <= 1);
    assert!(drift(folded.last_failed_at, t0 + Duration::seconds(3)) <= 1);
    assert_eq!(
        ledger.count_sync_failures().await.expect("count"),
        before + 3
    );

    // The limit is honoured.
    assert_eq!(ledger.list_sync_failures(1).await.expect("list").len(), 1);

    assert!(ledger.clear_sync_failure(&first.key).await.expect("clear"));
    assert!(
        !ledger.clear_sync_failure(&first.key).await.expect("clear"),
        "already gone"
    );
    for record in mine(ledger, tenant).await {
        assert!(ledger.clear_sync_failure(&record.key).await.expect("clear"));
    }
    assert!(mine(ledger, tenant).await.is_empty());
    assert_eq!(ledger.count_sync_failures().await.expect("count"), before);
}
