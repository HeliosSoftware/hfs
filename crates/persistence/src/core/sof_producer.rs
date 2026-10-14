//! The producer core of the SQL-on-FHIR SQL runners (#1864).
//!
//! A runner produces rows in a spawned task and hands the caller a
//! [`RowStream`]. This module is the part every SQL runner shares:
//!
//! - **A batch channel** ([`row_batch_channel`]). The producer collects rows
//!   into batches of up to [`BATCH_ROWS`] and sends whole batches; the
//!   consumer side flattens them back into single rows, so order,
//!   multiplicity and the position of an error item are exactly what the
//!   producer pushed. A producer flushes a partial batch before it waits on
//!   its backend, or once it is [`BATCH_LINGER`] old, so sparse rows are not
//!   held back until a batch fills. A columnar run
//!   ([`SofRunner::run_view_columnar`](super::sof_runner::SofRunner::run_view_columnar))
//!   uses the same channel with whole Arrow record batches as its items
//!   ([`record_batch_channel`]), and everything below applies to it as well.
//! - **An end marker.** The producer ends the stream explicitly
//!   ([`BatchSender::finish`]). If the channel closes without it (the task
//!   panicked or was cancelled), the consumer sees an error item, never a
//!   clean end, so a partial row set cannot look complete.
//! - **Supervision** ([`spawn_supervised`], [`spawn_blocking_supervised`],
//!   [`supervise`]). A task that ends with a `JoinError` is logged with its
//!   runner name, and its panic message reaches the consumer as the last item.
//! - **One stop signal** ([`RunStop`]): the consumer dropping the stream, the
//!   caller's [`RunContext::cancel`] token, or the run's deadline, whichever
//!   comes first, reported as a [`StopReason`]. A stop that is not a drop ends
//!   the stream with an error item ([`StopReason::into_error`]).
//! - **A bridge for blocking producers** ([`StopFlag`], [`bridge_stop`]): an
//!   async task waits for the stop, records its reason in an atomic flag the
//!   blocking producer polls, then interrupts the statement it is running.
//! - **Deadline helpers** ([`statement_timeout_ms`]): the time left, rounded
//!   up to whole milliseconds and clamped to `1..=i32::MAX`, so a
//!   `statement_timeout` derived from it never becomes `0` (which would
//!   disable the limit).

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use futures::Stream;
use helios_sof::arrow::record_batch::RecordBatch;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::{CancellationToken, DropGuard};
use tracing::warn;

use super::sof_runner::{BatchStream, RowStream, RunContext, SofError, StopReason, ViewRow};

/// Rows per batch sent through the channel.
pub const BATCH_ROWS: usize = 256;

/// Batches the channel queues ahead of the consumer.
pub const CHANNEL_BATCHES: usize = 4;

/// Arrow record batches the columnar channel queues ahead of the consumer
/// ([`record_batch_channel`]). One record batch already holds up to
/// [`helios_sof::columnar::COLUMNAR_BATCH_ROWS`] rows, so a couple of them is
/// plenty of read-ahead.
pub const CHANNEL_RECORD_BATCHES: usize = 2;

/// A partial batch older than this is sent even if the producer could add
/// more rows to it, so a sparse row reaches the consumer promptly.
pub const BATCH_LINGER: Duration = Duration::from_millis(50);

/// One batch of row items, in the order the producer pushed them.
pub type RowBatch = Vec<Result<ViewRow, SofError>>;

/// What travels through the channel.
enum Frame<T> {
    Items(Vec<Result<T, SofError>>),
    /// The producer finished its stream; nothing follows.
    End,
}

/// The producer half of a batch channel whose stream yields `T` items: rows
/// ([`BatchSender`]) or Arrow record batches ([`RecordBatchSender`]).
pub struct ItemSender<T> {
    tx: mpsc::Sender<Frame<T>>,
    pending: Vec<Result<T, SofError>>,
    pending_since: Option<std::time::Instant>,
    /// Items per frame: [`BATCH_ROWS`] rows, or one record batch.
    frame_items: usize,
}

/// The producer half of a row channel ([`row_batch_channel`]).
pub type BatchSender = ItemSender<ViewRow>;

/// The producer half of a columnar channel ([`record_batch_channel`]): each
/// item is one Arrow record batch, sent on its own.
pub type RecordBatchSender = ItemSender<RecordBatch>;

/// Reports a supervised task's failure on its channel (a sender clone kept
/// outside the task).
pub struct FailureReporter<T> {
    tx: mpsc::Sender<Frame<T>>,
}

impl<T> Clone for FailureReporter<T> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl<T: Send + 'static> FailureReporter<T> {
    async fn report(&self, error: SofError) {
        if self.tx.send(Frame::Items(vec![Err(error)])).await.is_ok() {
            let _ = self.tx.send(Frame::End).await;
        }
    }
}

impl<T: Send + 'static> ItemSender<T> {
    /// A reporter a supervisor uses to end this channel with an error item.
    pub fn reporter(&self) -> FailureReporter<T> {
        FailureReporter {
            tx: self.tx.clone(),
        }
    }

    /// Whether the consumer dropped the stream.
    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }

    /// Adds `item` to the pending batch. Returns `true` when the batch is full
    /// and should be flushed.
    pub fn push(&mut self, item: Result<T, SofError>) -> bool {
        if self.pending.is_empty() {
            self.pending_since = Some(std::time::Instant::now());
        }
        self.pending.push(item);
        self.pending.len() >= self.frame_items
    }

    /// Whether rows are waiting to be sent.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// How long the oldest unsent row has waited, if any.
    pub fn pending_age(&self) -> Option<Duration> {
        self.pending_since.map(|since| since.elapsed())
    }

    fn take_pending(&mut self) -> Vec<Result<T, SofError>> {
        self.pending_since = None;
        std::mem::take(&mut self.pending)
    }

    /// Sends the pending batch, waiting for room in the channel, raced
    /// against the stop: a slow consumer cannot hold the producer past a
    /// cancel or the deadline.
    pub async fn flush(&mut self, stop: &RunStop) -> Result<(), StopReason> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let permit = tokio::select! {
            biased;
            reason = stop.stopped() => return Err(reason),
            permit = self.tx.reserve() => permit.map_err(|_| StopReason::Dropped)?,
        };
        self.pending_since = None;
        permit.send(Frame::Items(std::mem::take(&mut self.pending)));
        Ok(())
    }

    /// [`flush`](Self::flush) for a blocking producer (a `spawn_blocking`
    /// thread), through `handle`.
    pub fn blocking_flush(
        &mut self,
        handle: &tokio::runtime::Handle,
        stop: &RunStop,
    ) -> Result<(), StopReason> {
        handle.block_on(self.flush(stop))
    }

    /// Sends the pending batch only if the channel has room right now. Never
    /// waits, so it can run inside a database callback. `Err` means the
    /// consumer is gone.
    pub fn try_flush(&mut self) -> Result<(), StopReason> {
        if self.pending.is_empty() {
            return Ok(());
        }
        match self.tx.try_reserve() {
            Ok(permit) => {
                self.pending_since = None;
                permit.send(Frame::Items(std::mem::take(&mut self.pending)));
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(())) => Ok(()),
            Err(mpsc::error::TrySendError::Closed(())) => Err(StopReason::Dropped),
        }
    }

    /// Sends what is pending, then `extra`, then the end marker, waiting only
    /// for a consumer that is still there.
    async fn close_with(mut self, stop: &RunStop, extra: Option<SofError>) {
        if let Some(error) = extra {
            self.push(Err(error));
        }
        let mut frames = Vec::with_capacity(2);
        if !self.pending.is_empty() {
            frames.push(Frame::Items(self.take_pending()));
        }
        frames.push(Frame::End);
        for frame in frames {
            let sent = tokio::select! {
                biased;
                _ = stop.dropped.cancelled() => false,
                sent = self.tx.send(frame) => sent.is_ok(),
            };
            if !sent {
                return;
            }
        }
    }

    /// Ends the stream normally: the pending rows, then the end marker.
    pub async fn finish(self, stop: &RunStop) {
        self.close_with(stop, None).await;
    }

    /// Ends the stream with `error` as its last item.
    pub async fn fail(self, stop: &RunStop, error: SofError) {
        self.close_with(stop, Some(error)).await;
    }

    /// Ends the stream after a stop: nothing for a drop, otherwise the stop's
    /// error item ([`StopReason::into_error`]).
    pub async fn end_after_stop(self, stop: &RunStop, reason: StopReason) {
        if let Some(error) = stop.error_for(reason) {
            self.fail(stop, error).await;
        }
    }

    /// [`finish`](Self::finish) for a blocking producer.
    pub fn blocking_finish(self, handle: &tokio::runtime::Handle, stop: &RunStop) {
        handle.block_on(self.finish(stop));
    }

    /// [`fail`](Self::fail) for a blocking producer.
    pub fn blocking_fail(self, handle: &tokio::runtime::Handle, stop: &RunStop, error: SofError) {
        handle.block_on(self.fail(stop, error));
    }

    /// [`end_after_stop`](Self::end_after_stop) for a blocking producer.
    pub fn blocking_end_after_stop(
        self,
        handle: &tokio::runtime::Handle,
        stop: &RunStop,
        reason: StopReason,
    ) {
        handle.block_on(self.end_after_stop(stop, reason));
    }
}

/// The consumer half: flattens batches into single items.
struct ItemStream<T> {
    rx: mpsc::Receiver<Frame<T>>,
    current: std::vec::IntoIter<Result<T, SofError>>,
    done: bool,
    runner: &'static str,
    /// Fires the stop signal's `dropped` token when the consumer drops the
    /// stream.
    _dropped: DropGuard,
}

// Nothing in the stream is pinned structurally: `poll_next` only reaches its
// fields through `&mut`, so moving it is fine whatever `T` is.
impl<T> Unpin for ItemStream<T> {}

impl<T> Stream for ItemStream<T> {
    type Item = Result<T, SofError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            if let Some(item) = self.current.next() {
                return Poll::Ready(Some(item));
            }
            if self.done {
                return Poll::Ready(None);
            }
            match self.rx.poll_recv(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Frame::Items(batch))) => self.current = batch.into_iter(),
                Poll::Ready(Some(Frame::End)) => {
                    self.done = true;
                    return Poll::Ready(None);
                }
                Poll::Ready(None) => {
                    // Every sender is gone and no end marker came: the
                    // producer died without saying why. Never a clean end.
                    self.done = true;
                    let runner = self.runner;
                    warn!(
                        runner,
                        "SoF row producer ended without finishing its stream"
                    );
                    return Poll::Ready(Some(Err(SofError::backend(format!(
                        "{runner}: row producer ended before finishing its stream"
                    )))));
                }
            }
        }
    }
}

/// One channel of `capacity` frames of up to `frame_items` items each.
fn item_channel<T>(
    runner: &'static str,
    ctx: &RunContext,
    capacity: usize,
    frame_items: usize,
) -> (ItemSender<T>, RunStop, ItemStream<T>) {
    let (tx, rx) = mpsc::channel(capacity);
    let stop = RunStop::new(ctx);
    let stream = ItemStream {
        rx,
        current: Vec::new().into_iter(),
        done: false,
        runner,
        _dropped: stop.dropped.clone().drop_guard(),
    };
    let sender = ItemSender {
        tx,
        pending: Vec::with_capacity(frame_items),
        pending_since: None,
        frame_items,
    };
    (sender, stop, stream)
}

/// Creates the channel of one run: the producer's [`BatchSender`], the run's
/// [`RunStop`] (from `ctx`, plus the consumer's drop) and the [`RowStream`]
/// the runner returns.
pub fn row_batch_channel(
    runner: &'static str,
    ctx: &RunContext,
) -> (BatchSender, RunStop, RowStream) {
    let (sender, stop, stream) = item_channel(runner, ctx, CHANNEL_BATCHES, BATCH_ROWS);
    (sender, stop, Box::pin(stream))
}

/// The columnar sibling of [`row_batch_channel`] (for
/// [`SofRunner::run_view_columnar`](super::sof_runner::SofRunner::run_view_columnar)):
/// the same end marker, supervision and stop signal, but each item is one
/// Arrow record batch, sent as soon as it is pushed.
pub fn record_batch_channel(
    runner: &'static str,
    ctx: &RunContext,
) -> (RecordBatchSender, RunStop, BatchStream) {
    let (sender, stop, stream) = item_channel(runner, ctx, CHANNEL_RECORD_BATCHES, 1);
    (sender, stop, Box::pin(stream))
}

/// Watches `tasks` and, if one ends with a `JoinError` (a panic or a
/// cancellation), logs it with `runner` and ends the stream with an error
/// item carrying the panic message. Tasks that end normally report nothing:
/// the producer ends the stream itself.
pub fn supervise<T: Send + 'static>(
    runner: &'static str,
    reporter: FailureReporter<T>,
    tasks: Vec<JoinHandle<()>>,
) {
    tokio::spawn(async move {
        let mut failure = None;
        for task in tasks {
            if let Err(e) = task.await {
                if failure.is_none() {
                    failure = Some(e);
                }
            }
        }
        if let Some(e) = failure {
            let message = join_error_message(&e);
            warn!(runner, error = %message, "SoF row producer ended before finishing its stream");
            reporter
                .report(SofError::backend(format!("row producer failed: {message}")))
                .await;
        }
    });
}

/// A `JoinError` as text, with the panic message when the task panicked.
fn join_error_message(e: &tokio::task::JoinError) -> String {
    e.to_string()
}

/// Spawns an async producer under [`supervise`].
pub fn spawn_supervised<T, F>(runner: &'static str, reporter: FailureReporter<T>, producer: F)
where
    T: Send + 'static,
    F: std::future::Future<Output = ()> + Send + 'static,
{
    supervise(runner, reporter, vec![tokio::spawn(producer)]);
}

/// Spawns a blocking producer under [`supervise`].
pub fn spawn_blocking_supervised<T, F>(
    runner: &'static str,
    reporter: FailureReporter<T>,
    producer: F,
) where
    T: Send + 'static,
    F: FnOnce() + Send + 'static,
{
    supervise(
        runner,
        reporter,
        vec![tokio::task::spawn_blocking(producer)],
    );
}

/// The stop signal of one run: the consumer's drop, the caller's cancel token
/// and the deadline.
#[derive(Debug, Clone)]
pub struct RunStop {
    cancel: CancellationToken,
    dropped: CancellationToken,
    deadline: Option<tokio::time::Instant>,
    budget: Option<Duration>,
}

impl RunStop {
    /// The stop signal for `ctx`, with a fresh drop token. The budget is the
    /// time left before the deadline when the run starts.
    pub fn new(ctx: &RunContext) -> Self {
        Self {
            cancel: ctx.cancel.clone(),
            dropped: CancellationToken::new(),
            deadline: ctx.deadline,
            budget: ctx
                .deadline
                .map(|d| d.saturating_duration_since(tokio::time::Instant::now())),
        }
    }

    /// The reason the run must stop now, if any. A drop wins over a cancel,
    /// and a cancel over the deadline.
    pub fn check(&self) -> Option<StopReason> {
        if self.dropped.is_cancelled() {
            Some(StopReason::Dropped)
        } else if self.cancel.is_cancelled() {
            Some(StopReason::Cancelled)
        } else if self.deadline_passed() {
            Some(StopReason::DeadlineExceeded)
        } else {
            None
        }
    }

    /// Resolves when the run must stop.
    pub async fn stopped(&self) -> StopReason {
        self.stopped_with_grace(Duration::ZERO).await
    }

    /// Like [`stopped`](Self::stopped), but the deadline counts only `grace`
    /// after it passed. For a step the backend itself bounds by the deadline
    /// (a statement under `SET LOCAL statement_timeout`): the client side is
    /// then only a backstop.
    pub async fn stopped_with_grace(&self, grace: Duration) -> StopReason {
        let deadline = async {
            match self.deadline {
                Some(deadline) => tokio::time::sleep_until(deadline + grace).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            biased;
            _ = self.dropped.cancelled() => StopReason::Dropped,
            _ = self.cancel.cancelled() => StopReason::Cancelled,
            _ = deadline => StopReason::DeadlineExceeded,
        }
    }

    /// Whether the consumer dropped the stream.
    pub fn is_dropped(&self) -> bool {
        self.dropped.is_cancelled()
    }

    /// The run's deadline.
    pub fn deadline(&self) -> Option<tokio::time::Instant> {
        self.deadline
    }

    /// The deadline as a [`std::time::Instant`], for a blocking producer.
    pub fn std_deadline(&self) -> Option<std::time::Instant> {
        self.deadline.map(tokio::time::Instant::into_std)
    }

    /// The time the run had when it started.
    pub fn budget(&self) -> Option<Duration> {
        self.budget
    }

    /// The time left before the deadline (zero once it passed), or `None`
    /// without a deadline.
    pub fn remaining(&self) -> Option<Duration> {
        self.deadline
            .map(|d| d.saturating_duration_since(tokio::time::Instant::now()))
    }

    /// Whether the deadline has passed.
    pub fn deadline_passed(&self) -> bool {
        self.deadline
            .is_some_and(|d| tokio::time::Instant::now() >= d)
    }

    /// The error item for `reason` ([`StopReason::into_error`] with this run's
    /// budget).
    pub fn error_for(&self, reason: StopReason) -> Option<SofError> {
        reason.into_error(self.budget)
    }
}

/// The stop reason of a run, readable from a blocking producer (and from a
/// database callback) without awaiting. The first reason set wins.
#[derive(Debug, Clone, Default)]
pub struct StopFlag(Arc<AtomicU8>);

impl StopFlag {
    const RUNNING: u8 = 0;

    /// Records `reason` unless a reason is already set.
    pub fn set(&self, reason: StopReason) {
        let code = match reason {
            StopReason::Dropped => 1,
            StopReason::Cancelled => 2,
            StopReason::DeadlineExceeded => 3,
        };
        let _ = self
            .0
            .compare_exchange(Self::RUNNING, code, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// The recorded reason, if any.
    pub fn get(&self) -> Option<StopReason> {
        match self.0.load(Ordering::SeqCst) {
            1 => Some(StopReason::Dropped),
            2 => Some(StopReason::Cancelled),
            3 => Some(StopReason::DeadlineExceeded),
            _ => None,
        }
    }
}

/// The stop bridge of a blocking producer: waits for `stop`, records its
/// reason in `flag` *before* it calls `on_stop` (which interrupts whatever the
/// producer is running), so the producer always finds the reason once it sees
/// the interrupt. Ends without doing anything once `done` fires (the producer
/// finished).
pub fn bridge_stop<F>(
    stop: RunStop,
    flag: StopFlag,
    done: CancellationToken,
    on_stop: F,
) -> JoinHandle<()>
where
    F: FnOnce() + Send + 'static,
{
    tokio::spawn(async move {
        tokio::select! {
            biased;
            _ = done.cancelled() => {}
            reason = stop.stopped() => {
                flag.set(reason);
                on_stop();
            }
        }
    })
}

/// A statement timeout in milliseconds for `remaining`: rounded up to whole
/// milliseconds, at least 1 (PostgreSQL reads `0` as "no limit") and at most
/// `i32::MAX` (PostgreSQL's maximum).
pub fn statement_timeout_ms(remaining: Duration) -> i32 {
    let ms = remaining.as_nanos().div_ceil(1_000_000);
    ms.clamp(1, i32::MAX as u128) as i32
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::StreamExt;
    use serde_json::json;

    use super::*;
    use crate::core::sof_runner::RunKind;

    fn ctx() -> RunContext {
        RunContext::detached(RunKind::Preview)
    }

    /// A10: whatever the batch boundaries, the consumer sees the producer's
    /// items in order, each once, with an error item in its place.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn batched_rows_flatten_in_order_with_errors_in_place() {
        let (mut out, stop, mut rows) = row_batch_channel("test", &ctx());
        let total = BATCH_ROWS * 3 + 7;
        let producer = tokio::spawn(async move {
            for i in 0..total {
                let item = if i == BATCH_ROWS + 3 {
                    Err(SofError::backend("in place"))
                } else {
                    Ok(json!({"n": i}))
                };
                if out.push(item) {
                    out.flush(&stop).await.unwrap();
                }
                // A few early flushes give batches of different sizes.
                if i % 97 == 0 {
                    out.flush(&stop).await.unwrap();
                }
            }
            out.finish(&stop).await;
        });
        let mut seen = Vec::new();
        while let Some(item) = rows.next().await {
            seen.push(item);
        }
        producer.await.unwrap();
        assert_eq!(seen.len(), total);
        for (i, item) in seen.iter().enumerate() {
            if i == BATCH_ROWS + 3 {
                assert!(
                    matches!(item, Err(SofError::Backend { .. })),
                    "{i}: {item:?}"
                );
            } else {
                assert_eq!(item.as_ref().unwrap(), &json!({"n": i}), "{i}");
            }
        }
    }

    /// A10: a producer that panics after some rows ends the stream with those
    /// rows and then one error item carrying the panic; never a clean end.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn panicking_batch_producer_yields_rows_then_error_item() {
        let (mut out, stop, mut rows) = row_batch_channel("test", &ctx());
        let reporter = out.reporter();
        spawn_blocking_supervised("test", reporter, move || {
            let handle = tokio::runtime::Handle::current();
            for i in 0..3 {
                out.push(Ok(json!({"n": i})));
            }
            out.blocking_flush(&handle, &stop).unwrap();
            panic!("boom");
        });
        let items = tokio::time::timeout(Duration::from_secs(10), async {
            let mut items = Vec::new();
            while let Some(item) = rows.next().await {
                items.push(item);
            }
            items
        })
        .await
        .expect("the stream ends");
        assert_eq!(items.len(), 4, "{items:?}");
        for (i, item) in items.iter().take(3).enumerate() {
            assert_eq!(item.as_ref().unwrap(), &json!({"n": i}));
        }
        match &items[3] {
            Err(SofError::Backend { message, .. }) => {
                assert!(message.contains("row producer failed"), "{message}")
            }
            other => panic!("expected a backend error item, got {other:?}"),
        }
    }

    /// A producer that drops its sender without the end marker (and without
    /// panicking) still cannot end the stream cleanly.
    #[tokio::test]
    async fn a_stream_without_its_end_marker_ends_with_an_error_item() {
        let (mut out, stop, mut rows) = row_batch_channel("test", &ctx());
        out.push(Ok(json!({"n": 0})));
        out.flush(&stop).await.unwrap();
        drop(out);
        assert!(rows.next().await.unwrap().is_ok());
        assert!(matches!(
            rows.next().await,
            Some(Err(SofError::Backend { .. }))
        ));
        assert!(rows.next().await.is_none());
    }

    /// Dropping the stream is a stop with no error item; a cancel and a
    /// deadline are stops that end the stream with their kind.
    #[tokio::test(start_paused = true)]
    async fn each_stop_has_its_reason_and_error_item() {
        let (_out, stop, rows) = row_batch_channel("test", &ctx());
        assert_eq!(stop.check(), None);
        drop(rows);
        assert_eq!(stop.check(), Some(StopReason::Dropped));
        assert_eq!(stop.stopped().await, StopReason::Dropped);
        assert!(stop.error_for(StopReason::Dropped).is_none());

        let ctx = ctx();
        let (_out, stop, _rows) = row_batch_channel("test", &ctx);
        ctx.cancel.cancel();
        assert_eq!(stop.stopped().await, StopReason::Cancelled);
        assert!(matches!(
            stop.error_for(StopReason::Cancelled),
            Some(SofError::Cancelled)
        ));

        let mut ctx = RunContext::detached(RunKind::Preview);
        ctx.deadline = Some(tokio::time::Instant::now() + Duration::from_secs(5));
        let (_out, stop, _rows) = row_batch_channel("test", &ctx);
        assert_eq!(stop.budget(), Some(Duration::from_secs(5)));
        let started = tokio::time::Instant::now();
        assert_eq!(stop.stopped().await, StopReason::DeadlineExceeded);
        assert_eq!(started.elapsed(), Duration::from_secs(5));
        assert!(matches!(
            stop.error_for(StopReason::DeadlineExceeded),
            Some(SofError::Timeout { budget: Some(b), .. }) if b == Duration::from_secs(5)
        ));
    }

    /// The deadline is a backstop for steps the server bounds: it fires only
    /// after the grace.
    #[tokio::test(start_paused = true)]
    async fn a_server_side_step_sees_the_deadline_after_its_grace() {
        let mut ctx = ctx();
        ctx.deadline = Some(tokio::time::Instant::now() + Duration::from_secs(1));
        let (_out, stop, _rows) = row_batch_channel("test", &ctx);
        let started = tokio::time::Instant::now();
        assert_eq!(
            stop.stopped_with_grace(Duration::from_secs(2)).await,
            StopReason::DeadlineExceeded
        );
        assert_eq!(started.elapsed(), Duration::from_secs(3));
    }

    /// A slow consumer cannot hold a producer past a cancel: the flush is
    /// raced against the stop.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_full_channel_does_not_hold_the_producer_past_a_cancel() {
        let ctx = ctx();
        let (mut out, stop, _rows) = row_batch_channel("test", &ctx);
        for _ in 0..CHANNEL_BATCHES {
            out.push(Ok(json!({})));
            out.flush(&stop).await.unwrap();
        }
        out.push(Ok(json!({})));
        let cancel = ctx.cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel.cancel();
        });
        let flushed = tokio::time::timeout(Duration::from_secs(5), out.flush(&stop))
            .await
            .expect("the flush returns once cancelled");
        assert_eq!(flushed, Err(StopReason::Cancelled));
    }

    /// The flag keeps the first reason, and the bridge sets it before it
    /// interrupts.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_bridge_records_the_reason_before_it_interrupts() {
        let ctx = ctx();
        let (_out, stop, _rows) = row_batch_channel("test", &ctx);
        let flag = StopFlag::default();
        let seen = Arc::new(std::sync::Mutex::new(None));
        let (seen_in, flag_in) = (seen.clone(), flag.clone());
        let bridge = bridge_stop(stop, flag.clone(), CancellationToken::new(), move || {
            *seen_in.lock().unwrap() = flag_in.get();
        });
        ctx.cancel.cancel();
        bridge.await.unwrap();
        assert_eq!(*seen.lock().unwrap(), Some(StopReason::Cancelled));
        flag.set(StopReason::DeadlineExceeded);
        assert_eq!(flag.get(), Some(StopReason::Cancelled));
    }

    /// R-b: a statement timeout from the time left is rounded up and never 0.
    #[test]
    fn statement_timeouts_round_up_and_stay_in_range() {
        assert_eq!(statement_timeout_ms(Duration::ZERO), 1);
        assert_eq!(statement_timeout_ms(Duration::from_nanos(1)), 1);
        assert_eq!(statement_timeout_ms(Duration::from_micros(1500)), 2);
        assert_eq!(statement_timeout_ms(Duration::from_millis(250)), 250);
        assert_eq!(
            statement_timeout_ms(Duration::from_millis(250) + Duration::from_nanos(1)),
            251
        );
        assert_eq!(
            statement_timeout_ms(Duration::from_secs(u64::MAX)),
            i32::MAX
        );
    }
}
