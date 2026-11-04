//! Staged, chunked creates for transaction Bundles (#1977).
//!
//! A create used to cost 3 to 7 commands inside the Bundle's transaction: a
//! `find_one` on `resources` that always missed, the `resources` insert, the
//! `resource_history` insert, two `delete_many` of search rows no create has
//! yet, and one or two `insert_many` of rows. The Bundle now stages its creates
//! and sends them in chunks:
//!
//! - **No existence read.** `idx_resources_identity` is unique and not partial
//!   on `(tenant_id, resource_type, id)`, and a soft-deleted resource keeps its
//!   document, so the index refuses exactly the ids the read used to find. A
//!   reused id comes back as a duplicate-key write error at the send and is
//!   reported as the same [`ResourceError::AlreadyExists`] at the same entry.
//! - **Chunks.** [`PendingCreates`] holds the creates an attempt has staged and
//!   not yet sent. A chunk is up to [`BUNDLE_CREATE_CHUNK_ENTRIES`] creates,
//!   [`BUNDLE_CREATE_CHUNK_BYTES`] of payload or [`SEARCH_INDEX_INSERT_CHUNK`]
//!   search rows. Each chunk is one ordered `insert_many` per collection, and
//!   for natively indexed creates one delete of their old rows per search
//!   collection ([`cleared_rows_filter`]): a created id has no rows of this
//!   Bundle, but a purge that failed half way can leave orphans for an id that
//!   is free again, so the delete stays, once per chunk.
//! - **When to send.** The loop in `storage.rs` decides, never entry
//!   processing, which for a create does no I/O. It sends before any entry
//!   that is not a plain POST (those read what earlier entries wrote), after
//!   the entry that fills the buffer, before it reports an entry failure, and
//!   after the last entry, before the deferred-index ledger and the commit.
//! - **Attribution.** A failed command is charged to the entry of the document
//!   the server refused, when the driver error names one, and otherwise to the
//!   first entry of the chunk being sent. Every earlier entry was fully
//!   written, so `failed_entry` keeps its meaning, but it can read up to
//!   [`BUNDLE_CREATE_CHUNK_ENTRIES`] - 1 entries low.
//!
//! The transaction writes the same documents and index keys as before, so
//! WiredTiger's per-transaction cache ceiling is not expected to move (inferred,
//! not measured on the benchmark): this shortens the time a Bundle runs, and
//! with it the time it holds the admission room, not the size it can reach.

use std::ops::Range;

use mongodb::ClientSession;
use mongodb::bson::{Document, doc};
use mongodb::error::{Error as MongoError, ErrorKind as MongoErrorKind};

use crate::error::{ResourceError, StorageError};

use super::MongoBackend;
use super::deferred_index::group_refs_by_type;
use super::storage::{
    SEARCH_INDEX_INSERT_CHUNK, SearchIndexDocuments, internal_driver_error, is_duplicate_key_error,
};

/// The most creates one chunk holds. A chunk of plain POSTs sends at most 6
/// commands: the `resources` and `resource_history` inserts, the two deletes of
/// old search rows and the two inserts of new ones. Before, each create sent 3
/// to 7 of its own. An abort position resolves to within 99 entries, 1.6% of the
/// 6.1k-entry median abort the #1776 benchmark measured.
pub(super) const BUNDLE_CREATE_CHUNK_ENTRIES: usize = 100;

/// The most payload one chunk holds, counted as the BSON wire size of the
/// payload in both copies (the `resources` document and its history copy). It
/// is not the memory held, which is larger: the `Document` trees, the
/// `missing_params` and the search rows, which are capped by count
/// ([`SEARCH_INDEX_INSERT_CHUNK`]) and not by bytes. It bounds what a Bundle of
/// large resources keeps beyond its own entries.
pub(super) const BUNDLE_CREATE_CHUNK_BYTES: usize = 16 * 1024 * 1024;

/// A create whose payload is at least this large is sent in a group of its own,
/// so a document the server refuses for its size is charged to its own entry
/// and not to the first of its chunk. 1 MiB is far above a typical resource.
pub(super) const BUNDLE_CREATE_ALONE_BYTES: usize = 1024 * 1024;

/// The server's duplicate-key code.
const DUPLICATE_KEY_CODE: i32 = 11000;

/// One create an attempt has staged: the documents to insert and what is needed
/// to charge a failure to the right entry.
pub(super) struct StagedCreate {
    /// The entry's position in the Bundle; a failure is charged to it.
    pub(super) entry: usize,
    pub(super) resource_type: String,
    pub(super) id: String,
    /// The `resources` document.
    pub(super) resource: Document,
    /// The version 1 `resource_history` document.
    pub(super) history: Document,
    /// BSON bytes of the payload held in each of the two documents.
    pub(super) payload_bytes: usize,
    /// The search rows to write in the transaction, after deleting the old
    /// ones: `Some` when the resource is natively indexed and its type is not
    /// deferred. `None` while search is offloaded or the Bundle defers the type
    /// (#1975), which write no rows in the transaction.
    pub(super) rows: Option<SearchIndexDocuments>,
}

/// The creates one attempt has staged and not yet sent. It starts empty with
/// every attempt, so an aborted or replayed attempt keeps nothing.
#[derive(Default)]
pub(super) struct PendingCreates {
    creates: Vec<StagedCreate>,
    /// Payload bytes of both copies of every staged create.
    bytes: usize,
    /// Search rows of every staged create, own and contained.
    rows: usize,
}

impl PendingCreates {
    pub(super) fn push(&mut self, create: StagedCreate) {
        self.bytes += 2 * create.payload_bytes;
        if let Some(rows) = &create.rows {
            self.rows += rows.own.len() + rows.contained.len();
        }
        self.creates.push(create);
    }

    pub(super) fn is_empty(&self) -> bool {
        self.creates.is_empty()
    }

    /// Whether a chunk is complete. The attempt sends the buffer once it is
    /// full, so a chunk goes over a cap by at most the create that reached it.
    pub(super) fn is_full(&self) -> bool {
        self.creates.len() >= BUNDLE_CREATE_CHUNK_ENTRIES
            || self.bytes >= BUNDLE_CREATE_CHUNK_BYTES
            || self.rows >= SEARCH_INDEX_INSERT_CHUNK
    }

    /// Hands over the staged creates and empties the buffer.
    pub(super) fn take(&mut self) -> Vec<StagedCreate> {
        self.bytes = 0;
        self.rows = 0;
        std::mem::take(&mut self.creates)
    }
}

/// How a flush splits `creates` into sends: maximal runs of consecutive
/// creates, except that a create of [`BUNDLE_CREATE_ALONE_BYTES`] or more is a
/// run of its own.
pub(super) fn send_groups(creates: &[StagedCreate]) -> Vec<Range<usize>> {
    let mut groups = Vec::new();
    let mut start = 0;
    for (at, create) in creates.iter().enumerate() {
        if create.payload_bytes >= BUNDLE_CREATE_ALONE_BYTES {
            if start < at {
                groups.push(start..at);
            }
            groups.push(at..at + 1);
            start = at + 1;
        }
    }
    if start < creates.len() {
        groups.push(start..creates.len());
    }
    groups
}

/// The position, in the documents of the `insert_many`, of the document the
/// server refused: the smallest index of its per-document errors. `None` for an
/// error that names no document (a failed command, a dropped connection).
pub(super) fn refused_document(err: &MongoError) -> Option<usize> {
    match err.kind.as_ref() {
        MongoErrorKind::InsertMany(insert) => insert
            .write_errors
            .as_ref()?
            .iter()
            .map(|error| error.index)
            .min(),
        _ => None,
    }
}

/// The position of the document an `insert_many` refused as a duplicate key
/// (code 11000), if it did.
pub(super) fn duplicate_id_at(err: &MongoError) -> Option<usize> {
    match err.kind.as_ref() {
        MongoErrorKind::InsertMany(insert) => insert
            .write_errors
            .as_ref()?
            .iter()
            .filter(|error| error.code == DUPLICATE_KEY_CODE)
            .map(|error| error.index)
            .min(),
        _ => None,
    }
}

/// The filter that selects the old search rows of the creates of one chunk:
/// per resource type, `{tenant_id, resource_type, resource_id: {$in: ids}}`,
/// and an `$or` of those when there is more than one type. Each clause is a
/// prefix of the `(tenant_id, resource_type, resource_id)` index of both search
/// collections. `refs` must not be empty: `$or` takes at least one clause.
pub(super) fn cleared_rows_filter(tenant_id: &str, refs: &[(String, String)]) -> Document {
    let mut clauses: Vec<Document> = group_refs_by_type(refs)
        .into_iter()
        .map(|(resource_type, ids)| {
            doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "resource_id": { "$in": ids },
            }
        })
        .collect();
    if clauses.len() == 1 {
        clauses.remove(0)
    } else {
        doc! { "$or": clauses }
    }
}

/// The entry a failed command is charged to: the one that owns the document the
/// error names, or the first of the group when it names none. `owners` maps a
/// document of the command to the position of its create in the group; `None`
/// when the documents are the creates themselves.
fn charged_entry(entries: &[usize], err: &MongoError, owners: Option<&[usize]>) -> usize {
    let position = refused_document(err)
        .and_then(|document| match owners {
            Some(owners) => owners.get(document).copied(),
            None => Some(document),
        })
        .unwrap_or(0);
    entries
        .get(position)
        .or_else(|| entries.first())
        .copied()
        .unwrap_or(0)
}

/// Maps the failure of a group's `resources` insert to the entry it is charged
/// to and its error.
///
/// A duplicate key is a reused id: [`ResourceError::AlreadyExists`] for the
/// refused create, or for the first of the group when the error names no
/// document but carries the server's duplicate-key text. Anything else keeps
/// the driver error as its source, so a `TransactionTooLargeForCache` (388), an
/// eviction rollback or a write conflict is classified as it is when any other
/// write of the Bundle fails with it.
///
/// `entries` and `identities` are the group's creates in send order.
pub(super) fn resource_insert_failure(
    entries: &[usize],
    identities: &[(String, String)],
    err: MongoError,
) -> (usize, StorageError) {
    let duplicate = duplicate_id_at(&err).or_else(|| {
        (refused_document(&err).is_none() && is_duplicate_key_error(&err)).then_some(0)
    });
    if let Some(at) = duplicate
        && let Some((resource_type, id)) = identities.get(at)
    {
        return (
            entries.get(at).copied().unwrap_or(0),
            StorageError::Resource(ResourceError::AlreadyExists {
                resource_type: resource_type.clone(),
                id: id.clone(),
            }),
        );
    }
    (
        charged_entry(entries, &err, None),
        internal_driver_error("Failed to insert resource in transaction", err),
    )
}

impl MongoBackend {
    /// Sends the staged creates in `pending`, in chunks, on the Bundle's
    /// session. On failure it returns the entry the failure is charged to and
    /// the error; the transaction is then the caller's to abort. The buffer is
    /// empty afterwards either way.
    pub(super) async fn flush_bundle_creates(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant_id: &str,
        pending: &mut PendingCreates,
    ) -> Result<(), (usize, StorageError)> {
        let creates = pending.take();
        if creates.is_empty() {
            return Ok(());
        }
        let groups = send_groups(&creates);
        let mut creates = creates.into_iter();
        for group in groups {
            let group: Vec<StagedCreate> = creates.by_ref().take(group.len()).collect();
            send_create_group(db, session, tenant_id, group).await?;
        }
        Ok(())
    }
}

/// Sends one group, in the order a create's own writes used to go: `resources`,
/// `resource_history`, the old search rows, then the new ones. Every
/// `insert_many` is ordered: it stops at the first document the server refuses,
/// so what is refused is deterministic, and inside a transaction any write
/// error aborts it anyway.
async fn send_create_group(
    db: &mongodb::Database,
    session: &mut ClientSession,
    tenant_id: &str,
    group: Vec<StagedCreate>,
) -> Result<(), (usize, StorageError)> {
    let mut entries = Vec::with_capacity(group.len());
    let mut identities = Vec::with_capacity(group.len());
    let mut resources = Vec::with_capacity(group.len());
    let mut histories = Vec::with_capacity(group.len());
    let mut cleared: Vec<(String, String)> = Vec::new();
    let mut own_rows: Vec<Document> = Vec::new();
    let mut own_owners: Vec<usize> = Vec::new();
    let mut contained_rows: Vec<Document> = Vec::new();
    let mut contained_owners: Vec<usize> = Vec::new();
    for (position, create) in group.into_iter().enumerate() {
        let StagedCreate {
            entry,
            resource_type,
            id,
            resource,
            history,
            rows,
            ..
        } = create;
        if let Some(rows) = rows {
            cleared.push((resource_type.clone(), id.clone()));
            own_owners.extend(std::iter::repeat_n(position, rows.own.len()));
            own_rows.extend(rows.own);
            contained_owners.extend(std::iter::repeat_n(position, rows.contained.len()));
            contained_rows.extend(rows.contained);
        }
        entries.push(entry);
        identities.push((resource_type, id));
        resources.push(resource);
        histories.push(history);
    }
    let first = entries.first().copied().unwrap_or(0);

    db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION)
        .insert_many(resources)
        .session(&mut *session)
        .await
        .map_err(|e| resource_insert_failure(&entries, &identities, e))?;

    db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION)
        .insert_many(histories)
        .session(&mut *session)
        .await
        .map_err(|e| {
            (
                charged_entry(&entries, &e, None),
                internal_driver_error("Failed to insert history in transaction", e),
            )
        })?;

    if !cleared.is_empty() {
        let filter = cleared_rows_filter(tenant_id, &cleared);
        db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION)
            .delete_many(filter.clone())
            .session(&mut *session)
            .await
            .map_err(|e| {
                (
                    first,
                    internal_driver_error("Failed to delete search index entries", e),
                )
            })?;
        db.collection::<Document>(MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION)
            .delete_many(filter)
            .session(&mut *session)
            .await
            .map_err(|e| {
                (
                    first,
                    internal_driver_error("Failed to delete search_index_contained entries", e),
                )
            })?;
    }

    if !own_rows.is_empty() {
        db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION)
            .insert_many(own_rows)
            .session(&mut *session)
            .await
            .map_err(|e| {
                (
                    charged_entry(&entries, &e, Some(&own_owners)),
                    internal_driver_error("Failed to insert search index entries", e),
                )
            })?;
    }
    if !contained_rows.is_empty() {
        db.collection::<Document>(MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION)
            .insert_many(contained_rows)
            .session(&mut *session)
            .await
            .map_err(|e| {
                (
                    charged_entry(&entries, &e, Some(&contained_owners)),
                    internal_driver_error("Failed to insert search_index_contained entries", e),
                )
            })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::{self, Bson};

    fn create(entry: usize, payload_bytes: usize, rows: Option<(usize, usize)>) -> StagedCreate {
        StagedCreate {
            entry,
            resource_type: "Patient".to_string(),
            id: format!("p{entry}"),
            resource: Document::new(),
            history: Document::new(),
            payload_bytes,
            rows: rows.map(|(own, contained)| SearchIndexDocuments {
                own: vec![Document::new(); own],
                contained: vec![Document::new(); contained],
                missing_params: None,
                missing_fp: None,
            }),
        }
    }

    /// A driver error for an `insert_many` the server refused, as the driver
    /// builds it from a reply with `writeErrors`.
    fn insert_many_error(errors: &[(usize, i32)]) -> MongoError {
        let write_errors: Vec<Bson> = errors
            .iter()
            .map(|(index, code)| {
                Bson::Document(doc! {
                    "index": *index as i32,
                    "code": *code,
                    "errmsg": "refused",
                })
            })
            .collect();
        let insert_many: mongodb::error::InsertManyError =
            bson::from_document(doc! { "writeErrors": write_errors })
                .expect("an InsertManyError deserializes from a server reply");
        MongoError::from(MongoErrorKind::InsertMany(insert_many))
    }

    fn command_error(code: i32) -> MongoError {
        let command: mongodb::error::CommandError = bson::from_document(doc! {
            "code": code,
            "codeName": "Failed",
            "errmsg": "refused",
        })
        .expect("a CommandError deserializes from a server error reply");
        MongoError::from(MongoErrorKind::Command(command))
    }

    #[test]
    fn an_empty_buffer_is_not_full() {
        let pending = PendingCreates::default();
        assert!(pending.is_empty());
        assert!(!pending.is_full());
    }

    #[test]
    fn the_buffer_is_full_at_its_entry_cap() {
        let mut pending = PendingCreates::default();
        for entry in 0..BUNDLE_CREATE_CHUNK_ENTRIES - 1 {
            pending.push(create(entry, 100, None));
            assert!(!pending.is_full(), "full after {} creates", entry + 1);
        }
        pending.push(create(BUNDLE_CREATE_CHUNK_ENTRIES - 1, 100, None));
        assert!(pending.is_full());
    }

    #[test]
    fn the_buffer_is_full_at_its_byte_cap() {
        // Both copies of the payload count, so half the cap per copy fills it.
        let mut pending = PendingCreates::default();
        pending.push(create(0, BUNDLE_CREATE_CHUNK_BYTES / 2 - 1, None));
        assert!(!pending.is_full());
        pending.push(create(1, 1, None));
        assert!(pending.is_full());
    }

    #[test]
    fn the_buffer_is_full_at_its_row_cap() {
        let mut pending = PendingCreates::default();
        pending.push(create(0, 100, Some((SEARCH_INDEX_INSERT_CHUNK - 3, 2))));
        assert!(!pending.is_full());
        // Contained rows count as well as own rows.
        pending.push(create(1, 100, Some((0, 1))));
        assert!(pending.is_full());
        // A create whose rows are not written in the transaction adds none.
        let mut pending = PendingCreates::default();
        pending.push(create(0, 100, None));
        assert!(!pending.is_full());
    }

    #[test]
    fn take_empties_the_buffer_and_its_counters() {
        let mut pending = PendingCreates::default();
        for entry in 0..BUNDLE_CREATE_CHUNK_ENTRIES {
            pending.push(create(entry, 100, Some((5, 1))));
        }
        assert!(pending.is_full());
        let taken = pending.take();
        assert_eq!(taken.len(), BUNDLE_CREATE_CHUNK_ENTRIES);
        assert_eq!(
            taken.iter().map(|c| c.entry).collect::<Vec<_>>(),
            (0..BUNDLE_CREATE_CHUNK_ENTRIES).collect::<Vec<_>>(),
            "creates come back in staging order"
        );
        assert!(pending.is_empty());
        assert!(!pending.is_full());
        // The counters restarted: one more create does not read as full.
        pending.push(create(0, 100, Some((5, 1))));
        assert!(!pending.is_full());
    }

    #[test]
    fn a_large_create_is_sent_in_a_group_of_its_own() {
        let creates: Vec<StagedCreate> = [10, 2 * 1024 * 1024, 10, 10]
            .into_iter()
            .enumerate()
            .map(|(entry, bytes)| create(entry, bytes, None))
            .collect();
        assert_eq!(send_groups(&creates), vec![0..1, 1..2, 2..4]);

        let creates: Vec<StagedCreate> = [BUNDLE_CREATE_ALONE_BYTES, BUNDLE_CREATE_ALONE_BYTES]
            .into_iter()
            .enumerate()
            .map(|(entry, bytes)| create(entry, bytes, None))
            .collect();
        assert_eq!(send_groups(&creates), vec![0..1, 1..2]);

        let creates: Vec<StagedCreate> = (0..3).map(|entry| create(entry, 10, None)).collect();
        assert_eq!(send_groups(&creates), vec![0..3]);
        assert!(send_groups(&[]).is_empty());
    }

    #[test]
    fn a_refused_document_is_found_by_its_index() {
        assert_eq!(
            refused_document(&insert_many_error(&[(7, 112), (3, 112), (9, 112)])),
            Some(3)
        );
        assert_eq!(refused_document(&insert_many_error(&[(0, 388)])), Some(0));
    }

    #[test]
    fn a_command_error_names_no_document() {
        let err = command_error(112);
        assert_eq!(refused_document(&err), None);
        assert_eq!(duplicate_id_at(&err), None);
        // An `insert_many` error without per-document errors names none either.
        let insert_many: mongodb::error::InsertManyError = bson::from_document(doc! {})
            .expect("an InsertManyError deserializes without writeErrors");
        let err = MongoError::from(MongoErrorKind::InsertMany(insert_many));
        assert_eq!(refused_document(&err), None);
    }

    #[test]
    fn a_duplicate_id_is_found_only_for_code_11000() {
        assert_eq!(duplicate_id_at(&insert_many_error(&[(4, 11000)])), Some(4));
        assert_eq!(duplicate_id_at(&insert_many_error(&[(4, 112)])), None);
        assert_eq!(duplicate_id_at(&insert_many_error(&[(4, 388)])), None);
    }

    #[test]
    fn the_cleared_rows_filter_keeps_each_type_with_its_own_ids() {
        let refs = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs
                .iter()
                .map(|(t, i)| (t.to_string(), i.to_string()))
                .collect()
        };

        let single = cleared_rows_filter("t1", &refs(&[("Patient", "a"), ("Patient", "b")]));
        assert_eq!(
            single,
            doc! {
                "tenant_id": "t1",
                "resource_type": "Patient",
                "resource_id": { "$in": ["a", "b"] },
            },
            "one type is a bare clause"
        );

        let two = cleared_rows_filter(
            "t1",
            &refs(&[("Patient", "a"), ("Observation", "a"), ("Patient", "b")]),
        );
        assert_eq!(
            two,
            doc! {
                "$or": [
                    { "tenant_id": "t1", "resource_type": "Patient",
                      "resource_id": { "$in": ["a", "b"] } },
                    { "tenant_id": "t1", "resource_type": "Observation",
                      "resource_id": { "$in": ["a"] } },
                ]
            },
            "each type keeps its own ids and carries the tenant"
        );
    }

    fn identities(count: usize) -> (Vec<usize>, Vec<(String, String)>) {
        (
            (10..10 + count).collect(),
            (0..count)
                .map(|n| ("Patient".to_string(), format!("p{n}")))
                .collect(),
        )
    }

    #[test]
    fn a_duplicate_in_a_chunk_is_charged_to_the_refused_create() {
        let (entries, ids) = identities(3);
        let (at, err) = resource_insert_failure(&entries, &ids, insert_many_error(&[(1, 11000)]));
        assert_eq!(at, 11);
        let StorageError::Resource(ResourceError::AlreadyExists { resource_type, id }) = err else {
            panic!("expected AlreadyExists, got {err:?}");
        };
        assert_eq!((resource_type.as_str(), id.as_str()), ("Patient", "p1"));
    }

    #[test]
    fn a_duplicate_key_without_an_index_is_charged_to_the_first_create() {
        let (entries, ids) = identities(3);
        let write: mongodb::error::WriteError = bson::from_document(doc! {
            "code": DUPLICATE_KEY_CODE,
            "codeName": "DuplicateKey",
            "errmsg": "E11000 duplicate key error collection: hfs.resources index: idx_resources_identity",
        })
        .expect("a WriteError deserializes from a server reply");
        let err = MongoError::from(MongoErrorKind::Write(
            mongodb::error::WriteFailure::WriteError(write),
        ));
        let (at, err) = resource_insert_failure(&entries, &ids, err);
        assert_eq!(at, 10);
        assert!(
            matches!(
                err,
                StorageError::Resource(ResourceError::AlreadyExists { ref id, .. }) if id == "p0"
            ),
            "{err:?}"
        );
    }

    #[test]
    fn any_other_refusal_keeps_the_driver_error_and_its_document() {
        let (entries, ids) = identities(3);
        let (at, err) = resource_insert_failure(&entries, &ids, insert_many_error(&[(2, 388)]));
        assert_eq!(at, 12);
        let StorageError::Backend(crate::error::BackendError::Internal {
            message, source, ..
        }) = err
        else {
            panic!("expected an internal error");
        };
        assert!(
            message.starts_with("Failed to insert resource in transaction: "),
            "{message}"
        );
        assert!(
            source.is_some(),
            "the driver error is kept for classification"
        );

        // A failed command names no document: the first create is charged.
        let (at, _) = resource_insert_failure(&entries, &ids, command_error(112));
        assert_eq!(at, 10);
    }

    #[test]
    fn a_row_failure_is_charged_to_the_create_that_owns_the_row() {
        let entries = [10, 11, 12];
        // Create 0 owns rows 0..2, create 2 owns rows 2..4.
        let owners = [0, 0, 2, 2];
        assert_eq!(
            charged_entry(&entries, &insert_many_error(&[(3, 112)]), Some(&owners)),
            12
        );
        assert_eq!(
            charged_entry(&entries, &insert_many_error(&[(1, 112)]), Some(&owners)),
            10
        );
        assert_eq!(
            charged_entry(&entries, &command_error(112), Some(&owners)),
            10
        );
    }
}
