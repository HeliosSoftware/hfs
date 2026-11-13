//! #2562: in-place metadata rewrites (`ResourceStorage::rewrite_meta`) and the
//! same-version revision (`StoredResource::meta_seq`) that guards every write
//! derived from a read. Shared by the SQL backends that serve rewrites.

use helios_fhir::FhirVersion;
use helios_persistence::core::{
    ConditionalDeleteResult, ConditionalStorage, ConditionalUpdateResult, EntityTagPrecondition,
    MetaChange, MetaTarget, ResourceStorage, VersionedStorage, delete_conditional_match,
    is_same_version_conflict,
};
use helios_persistence::error::{ResourceError, StorageError};
use helios_persistence::search::reindex::{ReindexSource, ReindexTarget};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::StoredResource;
use serde_json::{Value, json};

const TAGS: &str = "http://example.org/tags";

fn tenant(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

fn tag(code: &str) -> Value {
    json!({"system": TAGS, "code": code})
}

fn task(id: &str, tags: &[&str]) -> Value {
    let tags: Vec<Value> = tags.iter().map(|c| tag(c)).collect();
    json!({"resourceType": "Task", "id": id, "status": "requested", "intent": "order",
           "meta": {"tag": tags}})
}

fn codes(resource: &StoredResource) -> Vec<String> {
    resource.content()["meta"]["tag"]
        .as_array()
        .map(|tags| {
            tags.iter()
                .map(|t| t["code"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn add(code: &str) -> MetaChange {
    MetaChange::Add(json!({"tag": [tag(code)]}))
}

fn current_target(resource: &StoredResource) -> MetaTarget {
    MetaTarget::Current {
        expected_version: resource.version_id().to_string(),
        expected_meta_seq: resource.meta_seq(),
    }
}

async fn read<S: ResourceStorage>(storage: &S, tenant: &TenantContext, id: &str) -> StoredResource {
    storage
        .read(tenant, "Task", id)
        .await
        .unwrap()
        .expect("live resource")
}

async fn tagged<S: ConditionalStorage>(
    storage: &S,
    tenant: &TenantContext,
    code: &str,
) -> Vec<String> {
    storage
        .resolve_conditional_matches(tenant, "Task", &format!("_tag={TAGS}|{code}"))
        .await
        .unwrap()
        .iter()
        .map(|r| r.id().to_string())
        .collect()
}

/// T4: every write result carries the revision an immediate read sees, and
/// a version-advancing write sets it back to 0.
pub async fn counter_lifecycle<S>(storage: &S, tenant_id: &str)
where
    S: ResourceStorage + VersionedStorage,
{
    let tenant = tenant(tenant_id);
    let created = storage
        .create(&tenant, "Task", task("c1", &["a"]), FhirVersion::default())
        .await
        .unwrap();
    let agrees = |written: &StoredResource, read: &StoredResource| {
        assert_eq!(written.version_id(), read.version_id());
        assert_eq!(written.meta_seq(), read.meta_seq());
        assert_eq!(
            written.last_modified().timestamp_micros(),
            read.last_modified().timestamp_micros()
        );
    };
    agrees(&created, &read(storage, &tenant, "c1").await);
    assert_eq!(created.meta_seq(), 0);

    let rewritten = storage
        .rewrite_meta(&tenant, "Task", "c1", &current_target(&created), &add("b"))
        .await
        .unwrap();
    assert!(rewritten.current_changed);
    assert_eq!(rewritten.resource.meta_seq(), 1);
    agrees(&rewritten.resource, &read(storage, &tenant, "c1").await);

    let current = read(storage, &tenant, "c1").await;
    let updated = storage
        .update(&tenant, &current, task("c1", &["a", "b"]))
        .await
        .unwrap();
    assert_eq!(updated.meta_seq(), 0);
    agrees(&updated, &read(storage, &tenant, "c1").await);

    let history = storage
        .rewrite_meta(
            &tenant,
            "Task",
            "c1",
            &MetaTarget::Historical {
                version_id: "1".to_string(),
            },
            &add("h"),
        )
        .await
        .unwrap();
    assert!(!history.current_changed);
    let version = storage
        .vread(&tenant, "Task", "c1", "1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(codes(&version), ["a", "b", "h"]);
    assert_eq!(
        version.last_modified().timestamp_micros(),
        history.resource.last_modified().timestamp_micros()
    );
    assert_eq!(read(storage, &tenant, "c1").await.meta_seq(), 0);

    storage.delete(&tenant, "Task", "c1").await.unwrap();
    let restored = storage
        .restore_tombstone(&tenant, "Task", "c1", task("c1", &["r"]))
        .await
        .unwrap();
    assert_eq!(restored.meta_seq(), 0);
    agrees(&restored, &read(storage, &tenant, "c1").await);
    assert_eq!(codes(&read(storage, &tenant, "c1").await), ["r"]);

    // A live row is not a tombstone: the restore loses, nothing is written.
    let lost = storage
        .restore_tombstone(&tenant, "Task", "c1", task("c1", &["x"]))
        .await;
    assert!(
        matches!(
            lost,
            Err(StorageError::Resource(ResourceError::NotFound { .. })
                | StorageError::Concurrency(_))
        ),
        "{lost:?}"
    );
    assert_eq!(codes(&read(storage, &tenant, "c1").await), ["r"]);
}

/// T5: current row, history row and index change together; repeated
/// rewrites count up; a history-only rewrite leaves the current version
/// alone; an unknown version changes nothing.
pub async fn rewrite_consistency<S>(storage: &S, tenant_id: &str)
where
    S: ResourceStorage + VersionedStorage + ConditionalStorage,
{
    let tenant = tenant(tenant_id);
    let created = storage
        .create(&tenant, "Task", task("r1", &["a"]), FhirVersion::default())
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;

    let mut current = created.clone();
    for (round, code) in ["b", "c", "d"].into_iter().enumerate() {
        let rewrite = storage
            .rewrite_meta(&tenant, "Task", "r1", &current_target(&current), &add(code))
            .await
            .unwrap();
        assert_eq!(rewrite.resource.meta_seq(), round as u64 + 1);
        assert_eq!(rewrite.resource.version_id(), "1");
        assert!(rewrite.resource.last_modified() > current.last_modified());
        current = read(storage, &tenant, "r1").await;
        let version = storage
            .vread(&tenant, "Task", "r1", "1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(version.content(), current.content());
        assert_eq!(
            version.last_modified().timestamp_micros(),
            current.last_modified().timestamp_micros()
        );
        assert_eq!(tagged(storage, &tenant, code).await, ["r1"]);
    }
    assert_eq!(codes(&current), ["a", "b", "c", "d"]);

    let removed = storage
        .rewrite_meta(
            &tenant,
            "Task",
            "r1",
            &current_target(&current),
            &MetaChange::Delete(json!({"tag": [tag("a")]})),
        )
        .await
        .unwrap();
    assert_eq!(codes(&removed.resource), ["b", "c", "d"]);
    assert!(tagged(storage, &tenant, "a").await.is_empty());

    // A stale current target is refused, never redirected to history.
    let stale = storage
        .rewrite_meta(&tenant, "Task", "r1", &current_target(&created), &add("z"))
        .await;
    assert!(matches!(stale, Err(e) if is_same_version_conflict(&e)));

    let before = read(storage, &tenant, "r1").await;
    storage
        .update(&tenant, &before, task("r1", &["b", "c", "d"]))
        .await
        .unwrap();
    let current_before = read(storage, &tenant, "r1").await;
    storage
        .rewrite_meta(
            &tenant,
            "Task",
            "r1",
            &MetaTarget::Historical {
                version_id: "1".to_string(),
            },
            &add("old"),
        )
        .await
        .unwrap();
    let current_after = read(storage, &tenant, "r1").await;
    assert_eq!(current_after.content(), current_before.content());
    assert_eq!(current_after.meta_seq(), current_before.meta_seq());
    assert!(tagged(storage, &tenant, "old").await.is_empty());

    let missing = storage
        .rewrite_meta(
            &tenant,
            "Task",
            "r1",
            &MetaTarget::Historical {
                version_id: "9".to_string(),
            },
            &add("never"),
        )
        .await;
    assert!(matches!(
        missing,
        Err(StorageError::Resource(ResourceError::NotFound { .. }))
    ));
    assert_eq!(
        read(storage, &tenant, "r1").await.content(),
        current_after.content()
    );

    // A deleted resource's tombstone version is gone; its older versions
    // still have content.
    storage.delete(&tenant, "Task", "r1").await.unwrap();
    let tombstone = storage
        .rewrite_meta(
            &tenant,
            "Task",
            "r1",
            &MetaTarget::Historical {
                version_id: "3".to_string(),
            },
            &add("x"),
        )
        .await;
    assert!(matches!(
        tombstone,
        Err(StorageError::Resource(ResourceError::Gone { .. }))
    ));
    storage
        .rewrite_meta(
            &tenant,
            "Task",
            "r1",
            &MetaTarget::Historical {
                version_id: "2".to_string(),
            },
            &add("x"),
        )
        .await
        .unwrap();
}

/// T3 (storage half): a write that carries a selection made before a
/// same-version rewrite loses its compare-and-swap, and conditional writes
/// resolved afterwards no longer select a resource that lost its tag.
pub async fn stale_selection<S>(storage: &S, tenant_id: &str)
where
    S: ResourceStorage + ConditionalStorage,
{
    let tenant = tenant(tenant_id);
    storage
        .create(
            &tenant,
            "Task",
            task("s1", &["a", "keep"]),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let matched = storage
        .resolve_conditional_matches(&tenant, "Task", &format!("_tag={TAGS}|a"))
        .await
        .unwrap()
        .remove(0);
    assert_eq!(matched.meta_seq(), 0);

    storage
        .rewrite_meta(
            &tenant,
            "Task",
            "s1",
            &current_target(&matched),
            &MetaChange::Delete(json!({"tag": [tag("a")]})),
        )
        .await
        .unwrap();

    let updated = storage.update(&tenant, &matched, task("s1", &["a"])).await;
    assert!(matches!(updated, Err(e) if is_same_version_conflict(&e)));
    let deleted =
        delete_conditional_match(storage, &tenant, &EntityTagPrecondition::Absent, &matched).await;
    assert!(matches!(deleted, Err(e) if is_same_version_conflict(&e)));
    let pinned = storage
        .delete_at_revision(&tenant, "Task", "s1", "1", 0)
        .await;
    assert!(matches!(pinned, Err(e) if is_same_version_conflict(&e)));
    assert_eq!(codes(&read(storage, &tenant, "s1").await), ["keep"]);

    let criteria = format!("_tag={TAGS}|a");
    let update = storage
        .conditional_update(
            &tenant,
            "Task",
            task("s1", &["a"]),
            &criteria,
            false,
            FhirVersion::default(),
            &EntityTagPrecondition::Absent,
        )
        .await
        .unwrap();
    assert!(matches!(update, ConditionalUpdateResult::NoMatch));
    let delete = storage
        .conditional_delete(&tenant, "Task", &criteria, &EntityTagPrecondition::Absent)
        .await
        .unwrap();
    assert!(matches!(delete, ConditionalDeleteResult::NoMatch));
    assert_eq!(codes(&read(storage, &tenant, "s1").await), ["keep"]);

    // Conditional update by a criterion that still matches merges labels.
    let update = storage
        .conditional_update(
            &tenant,
            "Task",
            task("s1", &["new"]),
            &format!("_tag={TAGS}|keep"),
            false,
            FhirVersion::default(),
            &EntityTagPrecondition::Absent,
        )
        .await
        .unwrap();
    assert!(matches!(update, ConditionalUpdateResult::Updated(_)));
    assert_eq!(codes(&read(storage, &tenant, "s1").await), ["keep", "new"]);
}

/// T6: a reindex page fetched before a rewrite and written after it leaves
/// the rewrite's index entries in place.
pub async fn reindex_page_after_a_rewrite<S>(storage: &S, tenant_id: &str)
where
    S: ResourceStorage + ConditionalStorage + ReindexSource + ReindexTarget,
{
    let tenant = tenant(tenant_id);
    let created = storage
        .create(&tenant, "Task", task("x1", &["a"]), FhirVersion::default())
        .await
        .unwrap();
    let page = storage
        .fetch_resources_page(&tenant, "Task", None, 100)
        .await
        .unwrap();
    assert_eq!(page.resources.len(), 1);

    storage
        .rewrite_meta(
            &tenant,
            "Task",
            "x1",
            &current_target(&created),
            &MetaChange::Add(json!({"tag": [tag("b")]})),
        )
        .await
        .unwrap();

    for result in storage
        .write_search_entries_page(&tenant, &page.resources)
        .await
    {
        result.unwrap();
    }
    assert_eq!(tagged(storage, &tenant, "b").await, ["x1"]);
    assert_eq!(tagged(storage, &tenant, "a").await, ["x1"]);
}

/// A storage whose next criteria resolution is followed, before its caller
/// writes, by a metadata rewrite of what it selected.
struct RewriteAfterResolve<S> {
    inner: std::sync::Arc<S>,
    change: std::sync::Mutex<Option<MetaChange>>,
}

#[async_trait::async_trait]
impl<S> ResourceStorage for RewriteAfterResolve<S>
where
    S: ResourceStorage + ConditionalStorage + 'static,
{
    fn backend_name(&self) -> &'static str {
        "rewrite-after-resolve"
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> helios_persistence::error::StorageResult<StoredResource> {
        self.inner
            .create(tenant, resource_type, resource, fhir_version)
            .await
    }

    async fn create_or_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> helios_persistence::error::StorageResult<(StoredResource, bool)> {
        self.inner
            .create_or_update(tenant, resource_type, id, resource, fhir_version)
            .await
    }

    async fn read(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> helios_persistence::error::StorageResult<Option<StoredResource>> {
        self.inner.read(tenant, resource_type, id).await
    }

    async fn update(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> helios_persistence::error::StorageResult<StoredResource> {
        self.inner.update(tenant, current, resource).await
    }

    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> helios_persistence::error::StorageResult<()> {
        self.inner.delete(tenant, resource_type, id).await
    }

    async fn delete_versioned(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
    ) -> helios_persistence::error::StorageResult<()> {
        self.inner
            .delete_versioned(tenant, resource_type, id, expected_version)
            .await
    }

    async fn delete_at_revision(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
        expected_meta_seq: u64,
    ) -> helios_persistence::error::StorageResult<()> {
        self.inner
            .delete_at_revision(
                tenant,
                resource_type,
                id,
                expected_version,
                expected_meta_seq,
            )
            .await
    }

    fn tracks_meta_revision(&self) -> bool {
        self.inner.tracks_meta_revision()
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> helios_persistence::error::StorageResult<u64> {
        self.inner.count(tenant, resource_type).await
    }
}

#[async_trait::async_trait]
impl<S> ConditionalStorage for RewriteAfterResolve<S>
where
    S: ResourceStorage + ConditionalStorage + 'static,
{
    async fn resolve_conditional_matches(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params: &str,
    ) -> helios_persistence::error::StorageResult<Vec<StoredResource>> {
        let matches = self
            .inner
            .resolve_conditional_matches(tenant, resource_type, search_params)
            .await?;
        let change = self.change.lock().unwrap().take();
        if let (Some(change), [matched]) = (change, matches.as_slice()) {
            self.inner
                .rewrite_meta(
                    tenant,
                    resource_type,
                    matched.id(),
                    &current_target(matched),
                    &change,
                )
                .await
                .expect("the interloper's rewrite lands");
        }
        Ok(matches)
    }

    async fn conditional_create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        search_params: &str,
        fhir_version: FhirVersion,
    ) -> helios_persistence::error::StorageResult<helios_persistence::core::ConditionalCreateResult>
    {
        self.inner
            .conditional_create(tenant, resource_type, resource, search_params, fhir_version)
            .await
    }

    async fn conditional_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        search_params: &str,
        upsert: bool,
        fhir_version: FhirVersion,
        if_match: &EntityTagPrecondition,
    ) -> helios_persistence::error::StorageResult<ConditionalUpdateResult> {
        helios_persistence::core::conditional_update_resolving(
            self,
            tenant,
            resource_type,
            resource,
            search_params,
            upsert,
            fhir_version,
            if_match,
        )
        .await
    }

    async fn conditional_delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params: &str,
        if_match: &EntityTagPrecondition,
    ) -> helios_persistence::error::StorageResult<ConditionalDeleteResult> {
        helios_persistence::core::conditional_delete_resolving(
            self,
            tenant,
            resource_type,
            search_params,
            if_match,
        )
        .await
    }
}

/// T3 (retry branches): a metadata rewrite lands between a conditional
/// write's resolution and its write. The write loses its compare-and-swap,
/// resolves the criteria again and acts on what matches then: nothing, when
/// the rewrite removed the tag the criteria named; the rewritten resource,
/// labels merged, when it still matches.
pub async fn conditional_writes_resolve_again_after_a_rewrite<S>(
    storage: std::sync::Arc<S>,
    tenant_id: &str,
) where
    S: ResourceStorage + ConditionalStorage + 'static,
{
    let tenant = tenant(tenant_id);
    let interposed = RewriteAfterResolve {
        inner: storage.clone(),
        change: std::sync::Mutex::new(None),
    };
    let remove_a = || MetaChange::Delete(json!({"tag": [tag("a")]}));
    let criteria = format!("_tag={TAGS}|a");

    for id in ["u1", "d1", "k1"] {
        storage
            .create(
                &tenant,
                "Task",
                task(id, &["a", id]),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    *interposed.change.lock().unwrap() = Some(remove_a());
    let update = interposed
        .conditional_update(
            &tenant,
            "Task",
            task("u1", &["x"]),
            &format!("{criteria}&_id=u1"),
            false,
            FhirVersion::default(),
            &EntityTagPrecondition::Absent,
        )
        .await
        .unwrap();
    assert!(matches!(update, ConditionalUpdateResult::NoMatch));
    let u1 = read(&*storage, &tenant, "u1").await;
    assert_eq!(codes(&u1), ["u1"]);
    assert_eq!(u1.version_id(), "1", "nothing was written over the rewrite");

    *interposed.change.lock().unwrap() = Some(remove_a());
    let delete = interposed
        .conditional_delete(
            &tenant,
            "Task",
            &format!("{criteria}&_id=d1"),
            &EntityTagPrecondition::Absent,
        )
        .await
        .unwrap();
    assert!(matches!(delete, ConditionalDeleteResult::NoMatch));
    assert_eq!(codes(&read(&*storage, &tenant, "d1").await), ["d1"]);

    // Still matching after the rewrite: the second attempt updates it, with
    // the rewrite's label merged in.
    *interposed.change.lock().unwrap() = Some(add("late"));
    let update = interposed
        .conditional_update(
            &tenant,
            "Task",
            task("k1", &["x"]),
            &format!("{criteria}&_id=k1"),
            false,
            FhirVersion::default(),
            &EntityTagPrecondition::Absent,
        )
        .await
        .unwrap();
    assert!(matches!(update, ConditionalUpdateResult::Updated(_)));
    let k1 = read(&*storage, &tenant, "k1").await;
    assert_eq!(codes(&k1), ["a", "k1", "late", "x"]);
    assert_eq!(k1.version_id(), "2");
}
