//! Bounded queue in front of another audit sink (#2234).
//!
//! The middleware used to spawn one task per audited request, each writing
//! its own event. Nothing bounded those writes: under load they queued on the
//! audit connection pool, waited out its timeout and were lost. [`QueuedSink`]
//! puts a bounded channel between the request path and the store, drained by
//! a fixed number of writer tasks, so the store sees at most `writers`
//! concurrent writes no matter how many requests are in flight.
//!
//! When the channel is full, [`OverflowPolicy`] decides: `Block` makes the
//! caller wait for room (backpressure on the request), `Drop` discards the
//! event and counts it in `audit_events_dropped_total{reason="queue_full"}`.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use tokio::sync::{Mutex, Notify, mpsc};

use crate::fhir_model::AuditEvent;
use crate::metrics;
use crate::sink::AuditSink;

/// What a [`QueuedSink`] does with an event when its queue is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverflowPolicy {
    /// Wait for room. The audited request is held until its event is queued,
    /// so a slow store slows requests instead of losing events.
    #[default]
    Block,
    /// Discard the event and count it as dropped (`reason="queue_full"`).
    Drop,
}

impl fmt::Display for OverflowPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Block => write!(f, "block"),
            Self::Drop => write!(f, "drop"),
        }
    }
}

impl FromStr for OverflowPolicy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "block" => Ok(Self::Block),
            "drop" => Ok(Self::Drop),
            other => Err(format!(
                "invalid audit overflow policy '{other}': expected 'block' or 'drop'"
            )),
        }
    }
}

/// Sizing and overflow behavior of a [`QueuedSink`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueConfig {
    /// Events the queue holds before the overflow policy applies.
    pub capacity: usize,
    /// Writer tasks draining the queue: the most concurrent writes the inner
    /// sink sees. Defaults to [`default_writers`].
    pub writers: usize,
    /// Most events a writer hands to the inner sink's
    /// [`record_batch`](AuditSink::record_batch) at once.
    pub batch_size: usize,
    /// What to do when the queue is full.
    pub overflow: OverflowPolicy,
}

/// Four writers per available core, clamped to `4..=64`.
///
/// This matches the PostgreSQL pool's default size (four connections per
/// core), so the writers can keep the audit pool busy without queueing on it
/// long enough to hit its wait timeout. Fewer writers than the store can
/// absorb makes the queue the bottleneck: with four writers on eight cores a
/// CRUD load ran a third slower under `block` (#2234).
pub fn default_writers() -> usize {
    std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .saturating_mul(4)
        .clamp(4, 64)
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            capacity: 10_000,
            writers: default_writers(),
            batch_size: 64,
            overflow: OverflowPolicy::Block,
        }
    }
}

/// Counts events between acceptance and the end of their write, so
/// [`flush`](AuditSink::flush) can wait for the backlog to drain.
struct Backlog {
    pending: AtomicUsize,
    drained: Notify,
    sink_name: String,
}

impl Backlog {
    fn add(&self, n: usize) {
        let depth = self.pending.fetch_add(n, Ordering::AcqRel) + n;
        metrics::set_queue_depth(&self.sink_name, depth);
    }

    fn done(&self, n: usize) {
        let depth = self.pending.fetch_sub(n, Ordering::AcqRel) - n;
        metrics::set_queue_depth(&self.sink_name, depth);
        if depth == 0 {
            self.drained.notify_waiters();
        }
    }
}

/// An [`AuditSink`] that queues events for a fixed pool of writer tasks.
///
/// Must be created inside a Tokio runtime: [`new`](Self::new) spawns the
/// writers. They stop once the sink is dropped and the queue is empty.
pub struct QueuedSink {
    tx: mpsc::Sender<AuditEvent>,
    inner: Arc<dyn AuditSink>,
    backlog: Arc<Backlog>,
    overflow: OverflowPolicy,
    name: String,
}

impl QueuedSink {
    /// Wrap `inner` behind a queue sized by `config`. Zero `capacity`,
    /// `writers` or `batch_size` is raised to one.
    pub fn new(inner: Arc<dyn AuditSink>, config: QueueConfig) -> Self {
        let name = inner.name().to_string();
        let (tx, rx) = mpsc::channel(config.capacity.max(1));
        let rx = Arc::new(Mutex::new(rx));
        let backlog = Arc::new(Backlog {
            pending: AtomicUsize::new(0),
            drained: Notify::new(),
            sink_name: name.clone(),
        });
        metrics::set_queue_depth(&name, 0);

        for _ in 0..config.writers.max(1) {
            tokio::spawn(writer(
                Arc::clone(&rx),
                Arc::clone(&inner),
                Arc::clone(&backlog),
                config.batch_size.max(1),
            ));
        }

        Self {
            tx,
            inner,
            backlog,
            overflow: config.overflow,
            name,
        }
    }

    /// Events accepted and not yet written.
    pub fn pending(&self) -> usize {
        self.backlog.pending.load(Ordering::Acquire)
    }
}

/// Take a batch off the shared receiver and write it. The receiver lock is
/// held only while collecting, never across the write, so the writers'
/// writes overlap.
async fn writer(
    rx: Arc<Mutex<mpsc::Receiver<AuditEvent>>>,
    inner: Arc<dyn AuditSink>,
    backlog: Arc<Backlog>,
    batch_size: usize,
) {
    let mut batch = Vec::with_capacity(batch_size);
    loop {
        {
            let mut rx = rx.lock().await;
            if rx.recv_many(&mut batch, batch_size).await == 0 {
                return;
            }
        }
        // Settled on drop, so a panicking inner sink still releases its
        // events and cannot leave `flush` waiting forever.
        let _done = Settle {
            backlog: &backlog,
            n: batch.len(),
        };
        inner.record_batch(std::mem::take(&mut batch)).await;
    }
}

/// Marks a batch as finished when dropped.
struct Settle<'a> {
    backlog: &'a Backlog,
    n: usize,
}

impl Drop for Settle<'_> {
    fn drop(&mut self) {
        self.backlog.done(self.n);
    }
}

#[async_trait]
impl AuditSink for QueuedSink {
    async fn record(&self, event: AuditEvent) {
        // Counted before the send so a writer can never finish the event
        // before it is added.
        self.backlog.add(1);
        let result = match self.overflow {
            OverflowPolicy::Block => self.tx.send(event).await.map_err(|_| None),
            OverflowPolicy::Drop => self.tx.try_send(event).map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => Some(metrics::REASON_QUEUE_FULL),
                mpsc::error::TrySendError::Closed(_) => None,
            }),
        };
        if let Err(reason) = result {
            let reason = reason.unwrap_or(metrics::REASON_QUEUE_CLOSED);
            self.backlog.done(1);
            metrics::record_dropped(&self.name, reason, 1);
            tracing::error!(sink = %self.name, reason, "Audit event dropped before queueing");
        }
    }

    async fn record_batch(&self, events: Vec<AuditEvent>) {
        for event in events {
            self.record(event).await;
        }
    }

    /// Wait until every accepted event has been written, then flush the
    /// inner sink.
    async fn flush(&self) {
        loop {
            let drained = self.backlog.drained.notified();
            if self.pending() == 0 {
                break;
            }
            drained.await;
        }
        self.inner.flush().await;
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::AuditEventBuilder;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    /// Counts written events and the peak number of concurrent writes. Each
    /// write takes `delay`, or waits for `gate` when one is set.
    struct SlowSink {
        written: AtomicUsize,
        active: AtomicUsize,
        peak: AtomicUsize,
        delay: Duration,
        gate: Option<Arc<Notify>>,
    }

    impl SlowSink {
        fn new(delay: Duration) -> Self {
            Self {
                written: AtomicUsize::new(0),
                active: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
                delay,
                gate: None,
            }
        }
    }

    #[async_trait]
    impl AuditSink for SlowSink {
        async fn record(&self, _event: AuditEvent) {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            match &self.gate {
                Some(gate) => gate.notified().await,
                None => tokio::time::sleep(self.delay).await,
            }
            self.active.fetch_sub(1, Ordering::SeqCst);
            self.written.fetch_add(1, Ordering::SeqCst);
        }

        async fn flush(&self) {}

        fn name(&self) -> &str {
            "slow"
        }
    }

    fn event() -> AuditEvent {
        AuditEventBuilder::new("Device/hfs").build()
    }

    #[tokio::test]
    async fn flush_waits_for_every_event_and_bounds_concurrency() {
        let inner = Arc::new(SlowSink::new(Duration::from_millis(2)));
        let sink = Arc::new(QueuedSink::new(
            Arc::clone(&inner) as Arc<dyn AuditSink>,
            QueueConfig {
                capacity: 16,
                writers: 3,
                batch_size: 4,
                overflow: OverflowPolicy::Block,
            },
        ));

        // Many more concurrent producers than writers or queue slots.
        let producers: Vec<_> = (0..200)
            .map(|_| {
                let sink = Arc::clone(&sink);
                tokio::spawn(async move { sink.record(event()).await })
            })
            .collect();
        for p in producers {
            p.await.unwrap();
        }
        sink.flush().await;

        assert_eq!(inner.written.load(Ordering::SeqCst), 200);
        assert_eq!(sink.pending(), 0);
        assert!(inner.peak.load(Ordering::SeqCst) <= 3);
    }

    #[tokio::test]
    async fn drop_policy_discards_when_full() {
        let gate = Arc::new(Notify::new());
        let inner = Arc::new(SlowSink {
            gate: Some(Arc::clone(&gate)),
            ..SlowSink::new(Duration::ZERO)
        });
        let sink = QueuedSink::new(
            Arc::clone(&inner) as Arc<dyn AuditSink>,
            QueueConfig {
                capacity: 2,
                writers: 1,
                batch_size: 1,
                overflow: OverflowPolicy::Drop,
            },
        );

        // The first event is taken by the writer, which then parks on the gate.
        sink.record(event()).await;
        while inner.active.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        // Two fill the queue; the next three find it full.
        for _ in 0..5 {
            sink.record(event()).await;
        }
        assert_eq!(sink.pending(), 3);

        while inner.written.load(Ordering::SeqCst) < 3 {
            gate.notify_one();
            tokio::task::yield_now().await;
        }
        sink.flush().await;
        assert_eq!(inner.written.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn block_policy_waits_for_room() {
        let gate = Arc::new(Notify::new());
        let inner = Arc::new(SlowSink {
            gate: Some(Arc::clone(&gate)),
            ..SlowSink::new(Duration::ZERO)
        });
        let sink = Arc::new(QueuedSink::new(
            Arc::clone(&inner) as Arc<dyn AuditSink>,
            QueueConfig {
                capacity: 1,
                writers: 1,
                batch_size: 1,
                overflow: OverflowPolicy::Block,
            },
        ));

        sink.record(event()).await;
        while inner.active.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        sink.record(event()).await; // fills the queue
        let blocked = {
            let sink = Arc::clone(&sink);
            tokio::spawn(async move { sink.record(event()).await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!blocked.is_finished(), "a full queue must hold the caller");

        while inner.written.load(Ordering::SeqCst) < 3 {
            gate.notify_one();
            tokio::task::yield_now().await;
        }
        blocked.await.unwrap();
        sink.flush().await;
        assert_eq!(inner.written.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn overflow_policy_parses() {
        assert_eq!("block".parse(), Ok(OverflowPolicy::Block));
        assert_eq!(" DROP ".parse(), Ok(OverflowPolicy::Drop));
        assert!("spill".parse::<OverflowPolicy>().is_err());
    }
}
