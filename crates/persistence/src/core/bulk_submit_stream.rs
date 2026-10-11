//! The per-file NDJSON stream driver every bulk-submit engine runs.
//!
//! [`drive_ndjson_stream`] is what each backend's
//! [`StreamingBulkSubmitProvider::process_ndjson_stream`](super::bulk_submit::StreamingBulkSubmitProvider::process_ndjson_stream)
//! delegates to. It replaces the loop the four engines used to carry each —
//! read a line, parse it, push it, every `batch_size` entries `await` the
//! batch write — in which fetch, parse and write of one file never
//! overlapped. The driver pipelines them:
//!
//! * A **reader task** reads and parses the next batch while the current one
//!   writes. It stays at most one parsed batch ahead of the dispatcher, so a
//!   fast network never piles the whole file up in memory.
//! * Up to **`K` writer tasks** write batches of the same file concurrently
//!   (`K = min(options.batch_writers, engine cap)`, `HFS_BULK_SUBMIT_BATCH_WRITERS`).
//!   Engines whose batches must stay serial — SQLite's one write lock (#942)
//!   — keep the cap at one and still get the parse overlap.
//! * Results are applied in **stream order** through an apply log, whatever
//!   order the writers finish in, so the stream's counts, `lines_processed`,
//!   `max_errors` / `continue_on_error` decisions and cancellation behave
//!   exactly as the serial loop's did: cancellation stops admission and the
//!   batches in flight finish; a hard error stops admission and is returned
//!   once they have.
//!
//! # Why writes of one file may run concurrently at all
//!
//! Receipts are keyed by `(file, line)` and rollback records carry minted ids,
//! so they are order-independent. What is *not* order-independent is the
//! file watermark every engine keeps (`bulk_manifest_file_progress.max_line`
//! and its MongoDB and S3 equivalents): a batch charges only the lines above
//! the recorded watermark, and a reclaimed manifest resumes a file right
//! after it. If batch N+1 charged before batch N, N's lines would be under
//! the watermark — never counted, and after a crash never ingested. The
//! [`BatchSequencer`] therefore hands every batch a [`BatchTicket`] and makes
//! the **charge** step (watermark, manifest counters, and the `COMMIT` that
//! carries them where the engine ties the two) run in batch order through a
//! [`ChargeTurn`], while the writes before it run concurrently. A batch that
//! fails before completing its turn poisons the sequence: every later batch's
//! turn fails, it rolls back or stays uncharged, and the watermark remains the
//! last *contiguous* committed line, so a resume can never skip a batch that
//! was not committed.
//!
//! Two batches in flight never share a resource id: the dispatcher holds a
//! batch back until the one carrying the same `(type, id)` has finished, so an
//! engine without row locks (MongoDB, S3) cannot race two writes of one
//! resource, and the same-id ordering of the serial loop is preserved.
//! PostgreSQL additionally signals [`BatchTicket::began`] once its advisory
//! locks are held, and the dispatcher starts the next batch only then: locks
//! are acquired in batch order, so a later batch can never hold a key an
//! earlier one is still waiting for.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;

use super::bulk_submit::{
    BulkEntryOutcome, BulkEntryResult, BulkProcessingOptions, BulkSubmitProvider,
    CANCELLED_ABORT_REASON, NdjsonEntry, StreamProcessingResult, SubmissionId,
};
use crate::error::{BackendError, BulkSubmitError, StorageError, StorageResult};
use crate::tenant::TenantContext;

/// Parsed batches the reader task may hold ready ahead of the dispatcher, on
/// top of the one it is filling. One is enough: parsing is several times
/// faster than any engine's write, so a deeper buffer only costs memory.
const READ_AHEAD_BATCHES: usize = 1;

/// The abort reason a stream reports when a line's resource type contradicts
/// the manifest's and the options do not continue on error.
const MAX_ERRORS_ABORT_REASON: &str = "max errors exceeded";

fn internal_error(message: impl Into<String>) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "bulk-submit".to_string(),
        message: message.into(),
        source: None,
    })
}

// ============================================================================
// Sequencer
// ============================================================================

/// The per-file sequence the tickets share.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SequenceState {
    /// Every batch below this index has signalled [`BatchTicket::began`] (or
    /// finished, or failed). The dispatcher admits batch `n + 1` only once
    /// `next_begin > n`.
    next_begin: u64,
    /// The batch whose charge turn it is.
    next_charge: u64,
    /// The lowest batch index that failed before completing its charge.
    /// Once set, every later batch's turn is refused.
    failed_at: Option<u64>,
}

/// Orders the charge step of one file's batches (see the [module docs](self)).
#[derive(Clone, Debug)]
pub struct BatchSequencer {
    state: Arc<watch::Sender<SequenceState>>,
}

impl Default for BatchSequencer {
    fn default() -> Self {
        Self::new()
    }
}

impl BatchSequencer {
    /// A sequence with no batch admitted yet.
    pub fn new() -> Self {
        Self {
            state: Arc::new(watch::Sender::new(SequenceState::default())),
        }
    }

    /// The ticket of batch `seq`. Tickets must be issued in order, starting
    /// at zero, one per batch.
    pub fn ticket(&self, seq: u64) -> BatchTicket {
        BatchTicket {
            seq,
            shared: Some(Arc::clone(&self.state)),
            settled: false,
        }
    }

    /// Whether batch `seq` has signalled [`BatchTicket::began`], finished or
    /// failed — i.e. whether the next batch may be admitted.
    pub fn has_begun(&self, seq: u64) -> bool {
        let state = *self.state.borrow();
        state.next_begin > seq || state.failed_at.is_some()
    }

    /// Resolves once [`Self::has_begun`] holds for `seq`.
    pub async fn wait_began(&self, seq: u64) {
        let mut rx = self.state.subscribe();
        // The sender lives in `self`, so this cannot fail while we wait.
        let _ = rx
            .wait_for(|state| state.next_begin > seq || state.failed_at.is_some())
            .await;
    }

    /// The lowest batch that failed before completing its charge, if any.
    pub fn failed_at(&self) -> Option<u64> {
        self.state.borrow().failed_at
    }
}

fn mark_failed(state: &watch::Sender<SequenceState>, seq: u64) {
    state.send_modify(|state| {
        state.failed_at = Some(state.failed_at.map_or(seq, |failed| failed.min(seq)));
        state.next_begin = state.next_begin.max(seq + 1);
    });
}

/// One batch's place in its file's sequence.
///
/// Handed to [`BulkSubmitProvider::process_entries_ticketed`]. The engine
/// calls [`Self::began`] once it holds every lock the batch will hold, writes
/// the batch, then takes its [`ChargeTurn`] with [`Self::wait_charge`] for the
/// watermark and counter writes and [`ChargeTurn::complete`]s it. A ticket
/// dropped without a completed turn — the batch failed or was aborted before
/// it charged — poisons the sequence for every later batch.
///
/// [`Self::unsequenced`] is the standalone form: every wait returns at once
/// and nothing is poisoned. [`BulkSubmitProvider::process_entries`] uses it.
#[derive(Debug)]
pub struct BatchTicket {
    seq: u64,
    shared: Option<Arc<watch::Sender<SequenceState>>>,
    /// Set once the obligation to charge moved to a [`ChargeTurn`] or was
    /// refused, so `Drop` no longer poisons.
    settled: bool,
}

impl BatchTicket {
    /// A ticket that orders nothing: for a batch processed on its own.
    pub fn unsequenced() -> Self {
        Self {
            seq: 0,
            shared: None,
            settled: true,
        }
    }

    /// The batch's index in its file, starting at zero.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Whether this batch is part of a sequence, i.e. other batches of the
    /// same file may be in flight.
    pub fn is_sequenced(&self) -> bool {
        self.shared.is_some()
    }

    /// Signals that the batch holds every lock it will hold, so the next batch
    /// of the file may start. Engines without lock acquisition call it first
    /// thing. Taking or completing the turn implies it.
    pub fn began(&self) {
        if let Some(shared) = &self.shared {
            let seq = self.seq;
            shared.send_modify(|state| state.next_begin = state.next_begin.max(seq + 1));
        }
    }

    /// Waits for the batch's turn to charge. Fails — without a turn — when an
    /// earlier batch of the file failed before completing its own, so the
    /// caller must not charge: on an engine with transactions, roll back.
    pub async fn wait_charge(mut self) -> StorageResult<ChargeTurn> {
        let Some(shared) = self.shared.clone() else {
            return Ok(ChargeTurn {
                seq: self.seq,
                shared: None,
                completed: true,
            });
        };
        let seq = self.seq;
        let state = {
            let _wait = crate::perf::span(crate::perf::Phase::IngestChargeWait);
            let mut rx = shared.subscribe();
            // The sender is shared with this ticket, so it cannot close.
            *rx.wait_for(|state| {
                state.next_charge >= seq || state.failed_at.is_some_and(|failed| failed < seq)
            })
            .await
            .map_err(|_| internal_error("bulk-submit batch sequence closed"))?
        };
        if let Some(failed) = state.failed_at
            && failed < seq
        {
            // Dropping `self` below records this batch as not charged.
            return Err(internal_error(format!(
                "bulk-submit batch {seq} of this file was not charged: batch {failed} \
                 failed before its charge, so the file's watermark stops there"
            )));
        }
        self.settled = true;
        Ok(ChargeTurn {
            seq,
            shared: Some(shared),
            completed: false,
        })
    }
}

impl Drop for BatchTicket {
    fn drop(&mut self) {
        if !self.settled
            && let Some(shared) = &self.shared
        {
            mark_failed(shared, self.seq);
        }
    }
}

/// The batch's turn to charge its file watermark and manifest counters.
///
/// Dropped without [`Self::complete`] — the charge statements failed — it
/// poisons the sequence exactly as a dropped ticket does.
#[derive(Debug)]
pub struct ChargeTurn {
    seq: u64,
    shared: Option<Arc<watch::Sender<SequenceState>>>,
    completed: bool,
}

impl ChargeTurn {
    /// The batch's index in its file.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// The charge is durable: hands the turn to the next batch.
    pub fn complete(mut self) {
        self.completed = true;
        if let Some(shared) = &self.shared {
            let seq = self.seq;
            shared.send_modify(|state| {
                state.next_charge = state.next_charge.max(seq + 1);
                state.next_begin = state.next_begin.max(seq + 1);
            });
        }
    }
}

impl Drop for ChargeTurn {
    fn drop(&mut self) {
        if !self.completed
            && let Some(shared) = &self.shared
        {
            mark_failed(shared, self.seq);
        }
    }
}

// ============================================================================
// The writer the driver drives
// ============================================================================

/// What the stream driver needs of an engine: a batch write that honours its
/// [`BatchTicket`], and the engine's cap on concurrent batches of one file.
///
/// Every [`BulkSubmitProvider`] is one (through
/// [`BulkSubmitProvider::process_entries_ticketed`] and
/// [`BulkSubmitProvider::max_batch_writers`]); the trait exists so the driver
/// can be tested against a fake without a storage backend behind it.
#[async_trait]
pub trait StreamBatchWriter: Send + Sync {
    /// The most batches of one file this writer lets run concurrently.
    fn batch_writer_cap(&self) -> u32;

    /// Writes one batch; see [`BulkSubmitProvider::process_entries_ticketed`].
    async fn write_batch(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        entries: Vec<NdjsonEntry>,
        options: &BulkProcessingOptions,
        ticket: BatchTicket,
    ) -> StorageResult<Vec<BulkEntryResult>>;
}

#[async_trait]
impl<T: BulkSubmitProvider + ?Sized> StreamBatchWriter for T {
    fn batch_writer_cap(&self) -> u32 {
        self.max_batch_writers()
    }

    async fn write_batch(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
        entries: Vec<NdjsonEntry>,
        options: &BulkProcessingOptions,
        ticket: BatchTicket,
    ) -> StorageResult<Vec<BulkEntryResult>> {
        self.process_entries_ticketed(tenant, submission_id, manifest_id, entries, options, ticket)
            .await
    }
}

// ============================================================================
// Reader task
// ============================================================================

/// What the reader task hands the dispatcher, in stream order.
enum StreamEvent {
    /// A full batch, or the trailing partial one at end of stream;
    /// `line_number` is the line that completed it.
    Batch {
        entries: Vec<NdjsonEntry>,
        line_number: u64,
    },
    /// A line rejected before any batch saw it — malformed JSON, or a resource
    /// of the wrong type for this file — with the reason the stream reports if
    /// the options make it abort.
    Rejected {
        line_number: u64,
        abort_reason: String,
    },
    /// End of stream, after `line_number` lines.
    Eof { line_number: u64 },
    /// The reader failed; nothing after it was read.
    Failed(StorageError),
}

/// Reads, parses and batches the stream, one task per file.
async fn read_batches(
    mut reader: Box<dyn AsyncBufRead + Send + Unpin>,
    resource_type: String,
    batch_size: usize,
    resume_after_line: u64,
    tx: mpsc::Sender<StreamEvent>,
) {
    let mut line_number = 0u64;
    let mut batch = Vec::with_capacity(batch_size);
    let mut line = String::new();
    loop {
        line.clear();
        let bytes_read = match reader.read_line(&mut line).await {
            Ok(n) => n,
            Err(e) => {
                // #1127: surface the reader's own message (e.g. the
                // fetcher's give-up text) unprefixed so it reaches the
                // manifest's error artifact intact.
                let _ = tx
                    .send(StreamEvent::Failed(StorageError::BulkSubmit(
                        BulkSubmitError::InputStream {
                            message: e.to_string(),
                            source: Some(Box::new(e)),
                        },
                    )))
                    .await;
                return;
            }
        };
        if bytes_read == 0 {
            break;
        }
        line_number += 1;
        // Lines an earlier run of the manifest already committed (#1610).
        if line_number <= resume_after_line {
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed = {
            let _span = crate::perf::span(crate::perf::Phase::NdjsonParse);
            NdjsonEntry::parse(line_number, trimmed)
        };
        let event = match parsed {
            Ok(entry) if entry.resource_type != resource_type => Some(StreamEvent::Rejected {
                line_number,
                abort_reason: MAX_ERRORS_ABORT_REASON.to_string(),
            }),
            Ok(entry) => {
                batch.push(entry);
                (batch.len() >= batch_size).then(|| StreamEvent::Batch {
                    entries: std::mem::replace(&mut batch, Vec::with_capacity(batch_size)),
                    line_number,
                })
            }
            Err(e) => Some(StreamEvent::Rejected {
                line_number,
                abort_reason: format!("Parse error: {e}"),
            }),
        };
        if let Some(event) = event
            && tx.send(event).await.is_err()
        {
            // The dispatcher stopped listening: it is done with this file.
            return;
        }
    }
    if !batch.is_empty()
        && tx
            .send(StreamEvent::Batch {
                entries: batch,
                line_number,
            })
            .await
            .is_err()
    {
        return;
    }
    let _ = tx.send(StreamEvent::Eof { line_number }).await;
}

// ============================================================================
// Driver
// ============================================================================

/// Aborts the task when dropped, so a stream future dropped mid-file (lease
/// lost, shutdown) does not leave its reader running.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// What a writer task reports back.
struct BatchDone {
    seq: u64,
    line_number: u64,
    outcome: StorageResult<Vec<BulkEntryResult>>,
}

/// Everything a writer task borrows, shared through one `Arc`.
struct BatchContext<W> {
    writer: W,
    tenant: TenantContext,
    submission_id: SubmissionId,
    manifest_id: String,
    options: BulkProcessingOptions,
}

/// One entry of the apply log, in stream order.
enum LogEntry {
    /// Batch `seq`; applied once its writer has finished.
    Batch(u64),
    /// A rejected line, lying after `batches_before` batch events.
    Rejected {
        line_number: u64,
        abort_reason: String,
        batches_before: u64,
    },
    /// End of stream, lying after `batches_before` batch events.
    Eof {
        line_number: u64,
        batches_before: u64,
    },
}

/// The dispatcher's bookkeeping.
struct StreamState<'a> {
    options: &'a BulkProcessingOptions,
    resource_type: &'a str,
    result: StreamProcessingResult,
    /// Events in stream order, applied as their batches finish.
    log: VecDeque<LogEntry>,
    /// Finished batches not yet applied: `(line_number, results)` by `seq`.
    completed: HashMap<u64, (u64, Vec<BulkEntryResult>)>,
    /// The highest batch index admitted to a writer, if any.
    admitted_through: Option<u64>,
    /// The next batch index to apply: every batch below it is applied.
    next_to_apply: u64,
    /// Admission stopped: cancellation, an abort decision, or a failed batch.
    stopped: bool,
    /// Why the stream reports itself aborted, once decided.
    abort_reason: Option<String>,
    /// The error to return, with its position in the stream (the lowest wins).
    first_error: Option<(u64, StorageError)>,
    /// Clock for the phase report: reset after every applied batch, so each
    /// `ingest_progress` call covers the time since the previous one.
    batch_clock: Instant,
}

impl StreamState<'_> {
    fn error_count(&self) -> u64 {
        self.result.counts.error_count()
    }

    fn stop(&mut self, reason: impl Into<String>) {
        if self.abort_reason.is_none() {
            self.abort_reason = Some(reason.into());
        }
        self.stopped = true;
    }

    fn record_error(&mut self, position: u64, error: StorageError) {
        match &self.first_error {
            Some((earlier, _)) if *earlier <= position => {
                tracing::debug!(position, "bulk-submit stream: later batch error: {error}");
            }
            _ => self.first_error = Some((position, error)),
        }
        self.stopped = true;
    }

    /// Whether an event lying after `batches_before` batch events comes after
    /// the last admitted batch — past the file's watermark, so a stopped
    /// stream leaves it for the next run to walk again.
    fn past_admitted(&self, batches_before: u64) -> bool {
        self.admitted_through
            .is_none_or(|admitted| batches_before > admitted)
    }

    /// Applies every log entry the stream order allows: finished batches in
    /// order, and the rejected lines and end of stream between them. A
    /// stopped stream applies nothing past its last admitted batch.
    fn apply_ready(&mut self) {
        while let Some(front) = self.log.front() {
            match front {
                LogEntry::Batch(seq) => {
                    let seq = *seq;
                    let Some((line_number, results)) = self.completed.remove(&seq) else {
                        if self.stopped && self.past_admitted(seq) {
                            // Never admitted: the next run walks it again.
                            self.log.clear();
                        }
                        break;
                    };
                    self.next_to_apply = seq + 1;
                    self.result.lines_processed = self.result.lines_processed.max(line_number);
                    let walked = results.len();
                    for entry_result in results {
                        self.result.counts.increment(entry_result.outcome);
                    }
                    report_ingest_progress(self.resource_type, walked, &mut self.batch_clock);
                    if !self.stopped {
                        if !self.options.continue_on_error
                            && self.options.max_errors > 0
                            && self.error_count() >= self.options.max_errors as u64
                        {
                            self.stop(MAX_ERRORS_ABORT_REASON);
                        } else if self.options.is_cancelled() {
                            // Abort is cooperative: a claimed manifest checks
                            // between batches, so an aborted submission stops
                            // here with its partial counts intact instead of
                            // running to the end (#968).
                            self.stop(CANCELLED_ABORT_REASON);
                        }
                    }
                }
                LogEntry::Rejected {
                    line_number,
                    abort_reason,
                    batches_before,
                } => {
                    if self.stopped && self.past_admitted(*batches_before) {
                        self.log.clear();
                        break;
                    }
                    self.result.lines_processed = self.result.lines_processed.max(*line_number);
                    // Rejected here, so no batch will charge it to the
                    // manifest's counters; the worker adds it (#969).
                    self.result
                        .counts
                        .increment(BulkEntryOutcome::ValidationError);
                    self.result.unbatched_errors += 1;
                    if !self.stopped
                        && !self.options.continue_on_error
                        && (self.options.max_errors == 0
                            || self.error_count() >= self.options.max_errors as u64)
                    {
                        let reason = abort_reason.clone();
                        self.stop(reason);
                    }
                }
                LogEntry::Eof {
                    line_number,
                    batches_before,
                } => {
                    if self.stopped && self.past_admitted(*batches_before) {
                        self.log.clear();
                        break;
                    }
                    self.result.lines_processed = self.result.lines_processed.max(*line_number);
                }
            }
            self.log.pop_front();
        }
    }
}

/// Times one applied batch and hands it to the phase reporter (#947). The
/// clock read and reset run in every build; the reporting itself lives behind
/// [`crate::perf::log_ingest_progress`], which is a no-op without
/// `--cfg perf_phases`, so this stays a single unconditional call here.
fn report_ingest_progress(resource_type: &str, entries: usize, batch_started: &mut Instant) {
    let wall = batch_started.elapsed();
    *batch_started = Instant::now();
    crate::perf::log_ingest_progress(resource_type, entries as u64, wall);
}

/// The `(type, id)` keys of a batch's entries that carry a client id, for the
/// in-flight barrier. Entries without one get a server-minted id and can
/// collide with nothing.
fn batch_keys(entries: &[NdjsonEntry]) -> Vec<(String, String)> {
    entries
        .iter()
        .filter_map(|entry| {
            entry
                .resource_id
                .as_ref()
                .map(|id| (entry.resource_type.clone(), id.clone()))
        })
        .collect()
}

/// How many batches of one file the driver writes concurrently for `options`
/// on a writer with `cap`: the configured count bounded by the engine, and one
/// whenever the options can stop the stream mid-file — an error budget or
/// `continue_on_error = false` — so that "stop after the batch that hit the
/// cap" keeps meaning exactly that.
pub fn effective_batch_writers(options: &BulkProcessingOptions, cap: u32) -> usize {
    if options.max_errors > 0 || !options.continue_on_error {
        return 1;
    }
    options.batch_writers.max(1).min(cap.max(1)) as usize
}

/// Ingests one NDJSON file through `writer`: the shared body of every
/// engine's [`StreamingBulkSubmitProvider::process_ndjson_stream`](super::bulk_submit::StreamingBulkSubmitProvider::process_ndjson_stream).
/// See the [module docs](self) for the pipeline and its invariants.
///
/// `writer` is cloned into every writer task, which is why it must be
/// `Clone + 'static`; the backends are cheap handle clones.
pub async fn drive_ndjson_stream<W>(
    writer: W,
    tenant: &TenantContext,
    submission_id: &SubmissionId,
    manifest_id: &str,
    resource_type: &str,
    reader: Box<dyn AsyncBufRead + Send + Unpin>,
    options: &BulkProcessingOptions,
) -> StorageResult<StreamProcessingResult>
where
    W: StreamBatchWriter + Clone + Send + Sync + 'static,
{
    let mut state = StreamState {
        options,
        resource_type,
        result: StreamProcessingResult::new(),
        log: VecDeque::new(),
        completed: HashMap::new(),
        admitted_through: None,
        next_to_apply: 0,
        stopped: false,
        abort_reason: None,
        first_error: None,
        batch_clock: Instant::now(),
    };

    // An ingest cancelled before it read anything persists nothing.
    if options.is_cancelled() {
        return Ok(state.result.aborted(CANCELLED_ABORT_REASON));
    }
    crate::perf::mark_ingest_start();

    let writers = effective_batch_writers(options, writer.batch_writer_cap());
    let batch_size = options.batch_size.max(1) as usize;

    let (tx, mut rx) = mpsc::channel(READ_AHEAD_BATCHES);
    let mut reader_task = AbortOnDrop(tokio::spawn(read_batches(
        reader,
        resource_type.to_string(),
        batch_size,
        options.resume_after_line,
        tx,
    )));

    let context = Arc::new(BatchContext {
        writer,
        tenant: tenant.clone(),
        submission_id: submission_id.clone(),
        manifest_id: manifest_id.to_string(),
        options: options.clone(),
    });
    let sequencer = BatchSequencer::new();
    // Writer tasks; dropping the set aborts whatever is still running.
    let mut tasks: JoinSet<BatchDone> = JoinSet::new();
    // The `(type, id)` keys of every batch in flight, and each batch's share.
    let mut in_flight_keys: HashSet<(String, String)> = HashSet::new();
    let mut keys_of: HashMap<u64, Vec<(String, String)>> = HashMap::new();
    // The next batch to admit, parsed and waiting for a writer.
    let mut pending: Option<(u64, Vec<NdjsonEntry>, u64)> = None;
    let mut next_seq = 0u64;
    let mut reader_done = false;

    loop {
        // 1. Admit the pending batch when a writer is free, no batch in flight
        //    carries one of its ids, and the previous batch holds its locks.
        if let Some((seq, entries, line_number)) = pending.take() {
            if !state.stopped && options.is_cancelled() {
                // Abort is cooperative (#968): with several writers the
                // token is also checked before a batch is admitted, so no
                // batch starts writing once it is set.
                state.stop(CANCELLED_ABORT_REASON);
            }
            if state.stopped {
                // Never admitted: its lines stay above the watermark.
                drop(entries);
            } else {
                let keys = if writers > 1 {
                    batch_keys(&entries)
                } else {
                    Vec::new()
                };
                let collides = keys.iter().any(|key| in_flight_keys.contains(key));
                let previous_began = state
                    .admitted_through
                    .is_none_or(|previous| sequencer.has_begun(previous));
                // Bounded by the apply log too, not only by running tasks:
                // a head batch that stalls after its charge must not let the
                // file run arbitrarily far ahead of what is applied.
                let in_window = seq < state.next_to_apply + 2 * writers as u64;
                if tasks.len() < writers && !collides && previous_began && in_window {
                    in_flight_keys.extend(keys.iter().cloned());
                    keys_of.insert(seq, keys);
                    let ticket = sequencer.ticket(seq);
                    let context = Arc::clone(&context);
                    tasks.spawn(async move {
                        let outcome = context
                            .writer
                            .write_batch(
                                &context.tenant,
                                &context.submission_id,
                                &context.manifest_id,
                                entries,
                                &context.options,
                                ticket,
                            )
                            .await;
                        BatchDone {
                            seq,
                            line_number,
                            outcome,
                        }
                    });
                    state.admitted_through = Some(seq);
                    continue;
                }
                pending = Some((seq, entries, line_number));
            }
        }

        // 2. Wait for whatever can move things forward.
        let have_pending = pending.is_some();
        let want_event = !have_pending && !reader_done && !state.stopped;
        if tasks.is_empty() && !want_event {
            // Nothing in flight and nothing more to admit: a pending batch
            // here is impossible (a free writer, no keys in flight, and the
            // previous batch finished), so the stream is complete.
            debug_assert!(pending.is_none());
            break;
        }
        let waiting_on_began = have_pending
            && state
                .admitted_through
                .is_some_and(|previous| !sequencer.has_begun(previous));
        let previous = state.admitted_through.unwrap_or(0);
        let _slot_wait =
            have_pending.then(|| crate::perf::span(crate::perf::Phase::IngestSlotWait));
        tokio::select! {
            biased;
            Some(joined) = tasks.join_next(), if !tasks.is_empty() => {
                let done = match joined {
                    Ok(done) => done,
                    Err(join_error) => {
                        if join_error.is_panic() {
                            std::panic::resume_unwind(join_error.into_panic());
                        }
                        // Only the driver aborts its tasks, and only by
                        // dropping the set; a cancelled task here is a bug.
                        return Err(internal_error(format!(
                            "bulk-submit batch task ended unexpectedly: {join_error}"
                        )));
                    }
                };
                if let Some(keys) = keys_of.remove(&done.seq) {
                    for key in keys {
                        in_flight_keys.remove(&key);
                    }
                }
                match done.outcome {
                    Ok(results) => {
                        state.completed.insert(done.seq, (done.line_number, results));
                    }
                    Err(error) => state.record_error(done.seq, error),
                }
                state.apply_ready();
            }
            _ = sequencer.wait_began(previous), if waiting_on_began => {}
            event = rx.recv(), if want_event => {
                match event {
                    Some(StreamEvent::Batch { entries, line_number }) => {
                        let seq = next_seq;
                        next_seq += 1;
                        state.log.push_back(LogEntry::Batch(seq));
                        pending = Some((seq, entries, line_number));
                    }
                    Some(StreamEvent::Rejected { line_number, abort_reason }) => {
                        state.log.push_back(LogEntry::Rejected {
                            line_number,
                            abort_reason,
                            batches_before: next_seq,
                        });
                        state.apply_ready();
                    }
                    Some(StreamEvent::Eof { line_number }) => {
                        state.log.push_back(LogEntry::Eof {
                            line_number,
                            batches_before: next_seq,
                        });
                        reader_done = true;
                        state.apply_ready();
                    }
                    Some(StreamEvent::Failed(error)) => {
                        // Batches parsed before the failure still ingest, as
                        // they did when the loop hit the error after them;
                        // the error is returned once they are in.
                        reader_done = true;
                        match &state.first_error {
                            Some((earlier, _)) if *earlier <= next_seq => {}
                            _ => state.first_error = Some((next_seq, error)),
                        }
                    }
                    None => {
                        // The reader always ends with `Eof` or `Failed`; a
                        // channel closed without either means its task died
                        // (a panic in fetch, decompression or parse). The
                        // file's tail was never read, so this is not a clean
                        // end of file: fail the file once the batches already
                        // parsed are in, as `Failed` does.
                        reader_done = true;
                        let error = match (&mut reader_task.0).await {
                            Err(join_error) if join_error.is_panic() => {
                                let payload = join_error.into_panic();
                                let message = payload
                                    .downcast_ref::<&str>()
                                    .map(|s| (*s).to_string())
                                    .or_else(|| payload.downcast_ref::<String>().cloned())
                                    .unwrap_or_else(|| "unknown panic".to_string());
                                format!("the NDJSON reader panicked before the end of the file: {message}")
                            }
                            Err(join_error) => format!(
                                "the NDJSON reader ended before the end of the file: {join_error}"
                            ),
                            Ok(()) => "the NDJSON reader ended before the end of the file".to_string(),
                        };
                        let error = StorageError::BulkSubmit(BulkSubmitError::InputStream {
                            message: error,
                            source: None,
                        });
                        match &state.first_error {
                            Some((earlier, _)) if *earlier <= next_seq => {}
                            _ => state.first_error = Some((next_seq, error)),
                        }
                    }
                }
            }
        }
    }

    if let Some((_, error)) = state.first_error.take() {
        return Err(error);
    }
    let result = state.result;
    Ok(match state.abort_reason {
        Some(reason) => result.aborted(reason),
        None => result,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tenant::{TenantId, TenantPermissions};
    use std::sync::Mutex;
    use std::time::Duration;

    fn tenant() -> TenantContext {
        TenantContext::new(
            TenantId::new("stream-tests"),
            TenantPermissions::full_access(),
        )
    }

    fn ndjson(ids: impl IntoIterator<Item = String>) -> Box<dyn AsyncBufRead + Send + Unpin> {
        let text: String = ids
            .into_iter()
            .map(|id| format!("{{\"resourceType\":\"Patient\",\"id\":\"{id}\"}}\n"))
            .collect();
        Box::new(tokio::io::BufReader::new(std::io::Cursor::new(
            text.into_bytes(),
        )))
    }

    fn patients(n: usize) -> Box<dyn AsyncBufRead + Send + Unpin> {
        ndjson((1..=n).map(|i| format!("p{i}")))
    }

    /// What the fake writer remembers: the watermark model of a real engine
    /// (charge only lines above the recorded max line) plus the order things
    /// happened in.
    #[derive(Default, Debug)]
    struct Journal {
        /// Batches in the order their writes started / finished.
        started: Vec<u64>,
        finished: Vec<u64>,
        /// Batches in the order they charged.
        charged: Vec<u64>,
        /// The file watermark: highest line charged.
        max_line: u64,
        /// The lines every batch wrote, charged or not.
        written: Vec<u64>,
        /// `(started?, seq)` in the order writes started and finished.
        events: Vec<(bool, u64)>,
        /// Most batches writing at once.
        peak_concurrency: usize,
        in_flight: usize,
    }

    /// A writer whose batches take `delay(seq)`, fail at `fail_seq`, trip
    /// `cancel_on` when they start, and charge through a ticket.
    #[derive(Clone)]
    struct FakeWriter {
        cap: u32,
        journal: Arc<Mutex<Journal>>,
        delay: Arc<dyn Fn(u64) -> Duration + Send + Sync>,
        fail_seq: Option<u64>,
        cancel_on: Option<(u64, super::super::bulk_submit::CancelToken)>,
        /// Every entry's result is a processing error.
        all_fail: bool,
    }

    impl FakeWriter {
        fn new(cap: u32) -> Self {
            Self {
                cap,
                journal: Arc::new(Mutex::new(Journal::default())),
                delay: Arc::new(|_| Duration::ZERO),
                fail_seq: None,
                cancel_on: None,
                all_fail: false,
            }
        }

        fn with_delay(mut self, delay: impl Fn(u64) -> Duration + Send + Sync + 'static) -> Self {
            self.delay = Arc::new(delay);
            self
        }

        fn journal(&self) -> std::sync::MutexGuard<'_, Journal> {
            self.journal.lock().unwrap()
        }
    }

    #[async_trait]
    impl StreamBatchWriter for FakeWriter {
        fn batch_writer_cap(&self) -> u32 {
            self.cap
        }

        async fn write_batch(
            &self,
            _tenant: &TenantContext,
            _submission_id: &SubmissionId,
            _manifest_id: &str,
            entries: Vec<NdjsonEntry>,
            _options: &BulkProcessingOptions,
            ticket: BatchTicket,
        ) -> StorageResult<Vec<BulkEntryResult>> {
            let seq = ticket.seq();
            ticket.began();
            {
                let mut journal = self.journal();
                journal.started.push(seq);
                journal.events.push((true, seq));
                journal.in_flight += 1;
                journal.peak_concurrency = journal.peak_concurrency.max(journal.in_flight);
            }
            if let Some((on, token)) = &self.cancel_on
                && *on == seq
            {
                token.cancel();
            }
            tokio::time::sleep((self.delay)(seq)).await;
            let results: Vec<BulkEntryResult> = entries
                .iter()
                .map(|entry| {
                    if self.all_fail {
                        BulkEntryResult::processing_error(
                            entry.line_number,
                            &entry.resource_type,
                            serde_json::json!({"resourceType": "OperationOutcome"}),
                        )
                    } else {
                        BulkEntryResult::success(
                            entry.line_number,
                            &entry.resource_type,
                            entry.resource_id.clone().unwrap_or_default(),
                            true,
                        )
                    }
                })
                .collect();
            {
                let mut journal = self.journal();
                journal
                    .written
                    .extend(entries.iter().map(|e| e.line_number));
                journal.finished.push(seq);
                journal.events.push((false, seq));
                journal.in_flight -= 1;
            }
            if self.fail_seq == Some(seq) {
                return Err(internal_error(format!("batch {seq} exploded")));
            }
            let turn = ticket.wait_charge().await?;
            {
                let mut journal = self.journal();
                let last = entries.last().map(|e| e.line_number).unwrap_or(0);
                // A real engine charges only lines above its watermark.
                assert!(
                    last > journal.max_line,
                    "batch {seq} charged after a later batch: its lines are under the watermark"
                );
                journal.max_line = last;
                journal.charged.push(seq);
            }
            turn.complete();
            Ok(results)
        }
    }

    async fn drive(
        writer: &FakeWriter,
        reader: Box<dyn AsyncBufRead + Send + Unpin>,
        options: &BulkProcessingOptions,
    ) -> StorageResult<StreamProcessingResult> {
        drive_ndjson_stream(
            writer.clone(),
            &tenant(),
            &SubmissionId::generate("stream-tests"),
            "m1",
            "Patient",
            reader,
            options,
        )
        .await
    }

    #[tokio::test]
    async fn sequencer_orders_charges_whatever_order_batches_finish_in() {
        let sequencer = BatchSequencer::new();
        let t0 = sequencer.ticket(0);
        let t1 = sequencer.ticket(1);
        assert!(!sequencer.has_begun(0));
        t0.began();
        assert!(sequencer.has_begun(0));
        assert!(!sequencer.has_begun(1));

        // Batch 1 asks first; it only gets its turn after batch 0 completed.
        let later = tokio::spawn(async move {
            t1.wait_charge().await.map(|turn| {
                let seq = turn.seq();
                turn.complete();
                seq
            })
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!later.is_finished(), "batch 1 waits for batch 0's turn");
        let turn0 = t0.wait_charge().await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!later.is_finished(), "taking the turn is not completing it");
        turn0.complete();
        assert_eq!(later.await.unwrap().unwrap(), 1);
        assert!(sequencer.has_begun(1), "completing a turn implies began");
        assert_eq!(sequencer.failed_at(), None);
    }

    #[tokio::test]
    async fn sequencer_poisons_later_batches_when_an_earlier_one_fails() {
        let sequencer = BatchSequencer::new();
        let t0 = sequencer.ticket(0);
        let t1 = sequencer.ticket(1);
        let t2 = sequencer.ticket(2);
        // Batch 1 dies before charging: its ticket is dropped.
        drop(t1);
        assert_eq!(sequencer.failed_at(), Some(1));
        assert!(
            sequencer.has_begun(1),
            "a failed batch never blocks admission"
        );
        // Batch 0 is untouched: earlier than the failure.
        t0.wait_charge().await.unwrap().complete();
        // Batch 2 is refused, and refusing it records nothing lower.
        let err = t2.wait_charge().await.unwrap_err();
        assert!(err.to_string().contains("batch 1 failed"), "{err}");
        assert_eq!(sequencer.failed_at(), Some(1));

        // A turn dropped without `complete` poisons too.
        let sequencer = BatchSequencer::new();
        let turn = sequencer.ticket(0).wait_charge().await.unwrap();
        drop(turn);
        assert_eq!(sequencer.failed_at(), Some(0));
        assert!(sequencer.ticket(1).wait_charge().await.is_err());

        // An unsequenced ticket never waits and never poisons.
        let ticket = BatchTicket::unsequenced();
        assert!(!ticket.is_sequenced());
        ticket.wait_charge().await.unwrap().complete();
        drop(BatchTicket::unsequenced());
    }

    #[tokio::test]
    async fn batches_finish_out_of_order_and_charge_in_order() {
        // Batch 0 is the slowest, so 1..3 finish first; every batch must still
        // charge in file order, keeping the watermark contiguous.
        let writer =
            FakeWriter::new(8).with_delay(|seq| Duration::from_millis(40 - 10 * seq.min(3)));
        let options = BulkProcessingOptions::new()
            .with_batch_size(2)
            .with_batch_writers(4);
        let result = drive(&writer, patients(16), &options).await.unwrap();
        assert!(!result.aborted);
        assert_eq!(result.counts.success, 16);
        assert_eq!(result.lines_processed, 16);
        let journal = writer.journal();
        assert_eq!(journal.charged, (0..8).collect::<Vec<_>>());
        assert_eq!(journal.max_line, 16);
        assert!(journal.peak_concurrency > 1, "batches did run concurrently");
        assert!(
            journal.peak_concurrency <= 4,
            "{}",
            journal.peak_concurrency
        );
        assert_ne!(
            journal.finished, journal.charged,
            "the test needs out-of-order completion to prove anything"
        );
    }

    #[tokio::test]
    async fn a_failed_batch_stops_the_watermark_at_the_last_contiguous_commit() {
        // Batch 2 fails after batch 3 already wrote. Batch 3 must not charge:
        // the watermark stays at batch 1's last line, so a resume re-walks
        // batches 2 and 3 instead of skipping batch 2 forever.
        let writer = FakeWriter {
            fail_seq: Some(2),
            ..FakeWriter::new(8)
                .with_delay(|seq| Duration::from_millis(if seq == 2 { 30 } else { 1 }))
        };
        let options = BulkProcessingOptions::new()
            .with_batch_size(2)
            .with_batch_writers(4);
        let err = drive(&writer, patients(16), &options).await.unwrap_err();
        assert!(err.to_string().contains("batch 2 exploded"), "{err}");
        let journal = writer.journal();
        assert_eq!(journal.charged, vec![0, 1]);
        assert_eq!(journal.max_line, 4);
        assert!(
            journal.written.contains(&7),
            "batch 3 did write (it was in flight when batch 2 failed)"
        );
        assert!(
            !journal.started.contains(&6),
            "admission stopped: batch 6 was never started"
        );
    }

    #[tokio::test]
    async fn cancellation_stops_admission_and_lets_in_flight_batches_finish() {
        let cancel = super::super::bulk_submit::CancelToken::new();
        let writer = FakeWriter {
            cancel_on: Some((0, cancel.clone())),
            ..FakeWriter::new(8).with_delay(|_| Duration::from_millis(30))
        };
        let options = BulkProcessingOptions::new()
            .with_batch_size(2)
            .with_batch_writers(2)
            .with_cancel(cancel);
        let result = drive(&writer, patients(10), &options).await.unwrap();
        assert!(result.aborted);
        assert_eq!(result.abort_reason.as_deref(), Some(CANCELLED_ABORT_REASON));
        let journal = writer.journal();
        assert_eq!(
            journal.charged,
            vec![0],
            "the batch in flight when the token tripped finished and charged"
        );
        assert_eq!(result.counts.success, 2, "its entries are counted");
        assert_eq!(
            result.lines_processed, 2,
            "lines past it were never admitted"
        );
        assert_eq!(
            journal.started,
            vec![0],
            "no batch starts writing once the token is set, as in the serial loop"
        );
    }

    #[tokio::test]
    async fn a_single_writer_cancellation_keeps_the_serial_contract() {
        // The #968 contract on a serial engine: the batch already committed
        // when the token tripped is kept, nothing after it starts, and the
        // stream reports only the lines it admitted.
        let cancel = super::super::bulk_submit::CancelToken::new();
        let writer = FakeWriter {
            cancel_on: Some((0, cancel.clone())),
            ..FakeWriter::new(1)
        };
        let options = BulkProcessingOptions::new()
            .with_batch_size(2)
            .with_batch_writers(4)
            .with_cancel(cancel);
        let result = drive(&writer, patients(6), &options).await.unwrap();
        assert!(result.aborted);
        assert_eq!(result.counts.success, 2);
        assert_eq!(result.lines_processed, 2);
        assert_eq!(writer.journal().started, vec![0]);
    }

    #[tokio::test]
    async fn strict_options_abort_on_the_rejected_line_and_drop_the_partial_batch() {
        let writer = FakeWriter::new(8);
        let text = concat!(
            "{\"resourceType\":\"Patient\",\"id\":\"a\"}\n",
            "{\"resourceType\":\"Patient\",\"id\":\"b\"}\n",
            "not json\n",
            "{\"resourceType\":\"Patient\",\"id\":\"c\"}\n",
        );
        let reader: Box<dyn AsyncBufRead + Send + Unpin> = Box::new(tokio::io::BufReader::new(
            std::io::Cursor::new(text.as_bytes().to_vec()),
        ));
        let options = BulkProcessingOptions::new()
            .with_batch_size(2)
            .with_batch_writers(4)
            .with_continue_on_error(false);
        let result = drive(&writer, reader, &options).await.unwrap();
        assert!(result.aborted);
        assert!(
            result
                .abort_reason
                .as_deref()
                .is_some_and(|reason| reason.starts_with("Parse error: ")),
            "{:?}",
            result.abort_reason
        );
        assert_eq!(
            result.counts.success, 2,
            "the full batch before the bad line was written"
        );
        assert_eq!(result.counts.validation_error, 1);
        assert_eq!(result.unbatched_errors, 1);
        assert_eq!(result.lines_processed, 3, "reading stopped at the bad line");
        let journal = writer.journal();
        assert_eq!(journal.charged, vec![0]);
        assert_eq!(
            journal.peak_concurrency, 1,
            "strict options run one batch at a time"
        );
    }

    #[tokio::test]
    async fn an_error_budget_stops_after_the_batch_that_hit_it() {
        let writer = FakeWriter {
            all_fail: true,
            ..FakeWriter::new(8)
        };
        let options = BulkProcessingOptions::new()
            .with_batch_size(2)
            .with_batch_writers(4)
            .with_continue_on_error(false)
            .with_max_errors(2);
        let result = drive(&writer, patients(6), &options).await.unwrap();
        assert!(result.aborted);
        assert_eq!(result.abort_reason.as_deref(), Some("max errors exceeded"));
        assert_eq!(result.counts.processing_error, 2);
        assert_eq!(result.counts.total, 2);
        assert_eq!(result.lines_processed, 2);
        assert_eq!(writer.journal().started, vec![0]);
    }

    #[tokio::test]
    async fn wrong_type_lines_are_counted_and_the_stream_continues_by_default() {
        let writer = FakeWriter::new(8);
        let text = concat!(
            "{\"resourceType\":\"Patient\",\"id\":\"a\"}\n",
            "{\"resourceType\":\"Observation\",\"id\":\"o\"}\n",
            "\n",
            "garbage\n",
            "{\"resourceType\":\"Patient\",\"id\":\"b\"}\n",
        );
        let reader: Box<dyn AsyncBufRead + Send + Unpin> = Box::new(tokio::io::BufReader::new(
            std::io::Cursor::new(text.as_bytes().to_vec()),
        ));
        let options = BulkProcessingOptions::new()
            .with_batch_size(10)
            .with_batch_writers(4);
        let result = drive(&writer, reader, &options).await.unwrap();
        assert!(!result.aborted);
        assert_eq!(result.counts.success, 2);
        assert_eq!(result.counts.validation_error, 2);
        assert_eq!(result.unbatched_errors, 2);
        assert_eq!(result.lines_processed, 5);
        let journal = writer.journal();
        assert_eq!(
            journal.written,
            vec![1, 5],
            "the trailing partial batch was written at EOF"
        );
        assert_eq!(journal.charged, vec![0]);
    }

    #[tokio::test]
    async fn batches_sharing_a_resource_id_never_run_concurrently() {
        // p1 appears in batch 0 and batch 2: batch 2 must wait for batch 0 to
        // finish, while batch 1 (distinct ids) runs alongside batch 0.
        let writer = FakeWriter::new(8)
            .with_delay(|seq| Duration::from_millis(if seq == 0 { 40 } else { 1 }));
        let ids = ["p1", "p2", "p3", "p4", "p1", "p5"].map(String::from);
        let options = BulkProcessingOptions::new()
            .with_batch_size(2)
            .with_batch_writers(4);
        let result = drive(&writer, ndjson(ids), &options).await.unwrap();
        assert_eq!(result.counts.success, 6);
        let journal = writer.journal();
        assert_eq!(journal.charged, vec![0, 1, 2]);
        let at = |started: bool, seq: u64| {
            journal
                .events
                .iter()
                .position(|event| *event == (started, seq))
                .unwrap()
        };
        assert!(
            at(true, 2) > at(false, 0),
            "batch 2 started only after batch 0 finished: {:?}",
            journal.events
        );
        assert!(
            at(true, 1) < at(false, 0),
            "batch 1 ran alongside batch 0: {:?}",
            journal.events
        );
    }

    #[tokio::test]
    async fn resume_skips_the_committed_prefix_but_still_counts_its_lines() {
        let writer = FakeWriter::new(8);
        let options = BulkProcessingOptions::new()
            .with_batch_size(2)
            .with_batch_writers(4)
            .with_resume_after_line(3);
        let result = drive(&writer, patients(6), &options).await.unwrap();
        assert_eq!(result.counts.success, 3);
        assert_eq!(result.lines_processed, 6);
        let journal = writer.journal();
        assert_eq!(journal.written, vec![4, 5, 6]);
        assert_eq!(journal.max_line, 6);
    }

    #[tokio::test]
    async fn a_reader_failure_is_returned_after_the_batches_before_it_are_in() {
        struct BrokenAfter {
            inner: std::io::Cursor<Vec<u8>>,
            remaining: usize,
        }
        impl tokio::io::AsyncRead for BrokenAfter {
            fn poll_read(
                mut self: std::pin::Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>,
                buf: &mut tokio::io::ReadBuf<'_>,
            ) -> std::task::Poll<std::io::Result<()>> {
                if self.remaining == 0 {
                    return std::task::Poll::Ready(Err(std::io::Error::other(
                        "connection reset by provider",
                    )));
                }
                let n = self.remaining.min(buf.remaining());
                let pos = self.inner.position() as usize;
                let data = self.inner.get_ref()[pos..pos + n].to_vec();
                buf.put_slice(&data);
                self.inner.set_position((pos + n) as u64);
                self.remaining -= n;
                std::task::Poll::Ready(Ok(()))
            }
        }
        let text: String = (1..=4)
            .map(|i| format!("{{\"resourceType\":\"Patient\",\"id\":\"p{i}\"}}\n"))
            .collect();
        let bytes = text.into_bytes();
        // Exactly the first two lines are readable.
        let readable = bytes
            .iter()
            .enumerate()
            .filter(|(_, b)| **b == b'\n')
            .nth(1)
            .unwrap()
            .0
            + 1;
        let reader: Box<dyn AsyncBufRead + Send + Unpin> =
            Box::new(tokio::io::BufReader::new(BrokenAfter {
                inner: std::io::Cursor::new(bytes),
                remaining: readable,
            }));
        let writer = FakeWriter::new(8);
        let options = BulkProcessingOptions::new()
            .with_batch_size(2)
            .with_batch_writers(4);
        let err = drive(&writer, reader, &options).await.unwrap_err();
        assert_eq!(err.to_string(), "connection reset by provider");
        assert_eq!(
            writer.journal().charged,
            vec![0],
            "the batch before the break was written"
        );
    }

    #[tokio::test]
    async fn cancelled_before_the_first_line_reads_nothing() {
        let cancel = super::super::bulk_submit::CancelToken::new();
        cancel.cancel();
        let writer = FakeWriter::new(8);
        let options = BulkProcessingOptions::new().with_cancel(cancel);
        let result = drive(&writer, patients(4), &options).await.unwrap();
        assert!(result.aborted);
        assert_eq!(result.lines_processed, 0);
        assert!(writer.journal().started.is_empty());
    }

    #[test]
    fn effective_writers_honour_the_cap_and_the_strict_options() {
        let options = BulkProcessingOptions::new().with_batch_writers(4);
        assert_eq!(effective_batch_writers(&options, 1), 1);
        assert_eq!(effective_batch_writers(&options, 8), 4);
        assert_eq!(effective_batch_writers(&options, 2), 2);
        assert_eq!(
            effective_batch_writers(&options.clone().with_max_errors(1), 8),
            1
        );
        assert_eq!(
            effective_batch_writers(&options.clone().with_continue_on_error(false), 8),
            1
        );
        assert_eq!(
            effective_batch_writers(&BulkProcessingOptions::new(), 8),
            1,
            "the engine default is one writer"
        );
        assert_eq!(
            effective_batch_writers(&options.with_batch_writers(0), 8),
            1
        );
    }
}

#[cfg(test)]
#[path = "bulk_submit_stream_pipeline_tests.rs"]
mod pipeline_tests;
