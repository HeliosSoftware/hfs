//! Volatile, bounded dispatch service. Only the outer engine owns its sender.
use super::EngineCore;
use crate::manager::{GenerationToken, SubscriptionSnapshot};
use crate::{ResourceEvent, SubscriptionConfig};
use std::collections::{HashMap, VecDeque};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch};
use tracing::warn;

type Key = (String, String);
fn key(snapshot: &SubscriptionSnapshot) -> Key {
    (
        snapshot.subscription.tenant_id.clone(),
        snapshot.subscription.id.clone(),
    )
}

/// Effective limits, coherent occupancy, and cumulative overload counters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchSnapshot {
    pub event_queue_capacity: usize,
    pub max_pending_notifications: usize,
    pub per_subscription_queue_capacity: usize,
    pub dispatch_concurrency: usize,
    pub handshake_concurrency: usize,
    pub ingress_queued: usize,
    pub evaluating_events: usize,
    pub resident_notifications: usize,
    pub queued_notifications: usize,
    pub active_notifications: usize,
    pub notification_workers: usize,
    pub active_attempts: usize,
    pub pending_activations: usize,
    pub active_activations: usize,
    pub dropped_ingress_full: u64,
    pub dropped_ingress_closed: u64,
    pub dropped_global_full: u64,
    pub dropped_subscription_full: u64,
    pub dropped_worker_full: u64,
}

struct Completion {
    pending: AtomicUsize,
    sender: Mutex<Option<oneshot::Sender<()>>>,
}
pub(super) struct CompletionTicket(Arc<Completion>);
impl CompletionTicket {
    fn new() -> (Self, oneshot::Receiver<()>) {
        let (sender, receiver) = oneshot::channel();
        (
            Self(Arc::new(Completion {
                pending: AtomicUsize::new(1),
                sender: Mutex::new(Some(sender)),
            })),
            receiver,
        )
    }
}
impl Clone for CompletionTicket {
    fn clone(&self) -> Self {
        self.0.pending.fetch_add(1, Ordering::Relaxed);
        Self(Arc::clone(&self.0))
    }
}
impl Drop for CompletionTicket {
    fn drop(&mut self) {
        if self.0.pending.fetch_sub(1, Ordering::AcqRel) == 1 {
            if let Some(sender) = self
                .0
                .sender
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
            {
                let _ = sender.send(());
            }
        }
    }
}
struct EventWork {
    event: ResourceEvent,
    completion: Option<CompletionTicket>,
    ingress: IngressGuard,
}
struct IngressGuard {
    state: Weak<SchedulerState>,
    permit: Option<OwnedSemaphorePermit>,
}
impl IngressGuard {
    fn begin_evaluation(mut self, state: &Arc<SchedulerState>) -> EvaluationGuard {
        {
            let mut inner = state.lock();
            inner.stats.ingress_queued -= 1;
            inner.stats.evaluating_events += 1;
        }
        // recv() already returned mpsc capacity. Retain our ingress slot until
        // the queued/evaluating transition is visible as one coherent snapshot.
        self.state = Weak::new();
        self.permit.take();
        EvaluationGuard {
            state: Arc::clone(state),
        }
    }
}
impl Drop for IngressGuard {
    fn drop(&mut self) {
        if let Some(state) = self.state.upgrade() {
            state.lock().stats.ingress_queued -= 1;
        }
    }
}
struct EvaluationGuard {
    state: Arc<SchedulerState>,
}
impl Drop for EvaluationGuard {
    fn drop(&mut self) {
        self.state.lock().stats.evaluating_events -= 1;
    }
}

pub(super) struct ResidentGuard {
    state: Weak<SchedulerState>,
    key: Key,
    generation: GenerationToken,
    active: bool,
}
impl ResidentGuard {
    fn activate(&mut self, inner: &mut Inner) {
        inner.stats.queued_notifications -= 1;
        inner.stats.active_notifications += 1;
        self.active = true;
    }
}
impl Drop for ResidentGuard {
    fn drop(&mut self) {
        if let Some(state) = self.state.upgrade() {
            let mut inner = state.lock();
            inner.stats.resident_notifications -= 1;
            if self.active {
                inner.stats.active_notifications -= 1;
            } else {
                inner.stats.queued_notifications -= 1;
            }
            if let Some(queue) = inner
                .queues
                .get_mut(&self.key)
                .filter(|q| q.generation.same_generation(&self.generation))
            {
                queue.resident -= 1;
                if queue.resident == 0 && !queue.worker {
                    inner.queues.remove(&self.key);
                }
            }
        }
    }
}
struct DeliveryJob {
    snapshot: SubscriptionSnapshot,
    bundle: serde_json::Value,
    event_number: u64,
    resident: ResidentGuard,
    _completion: Option<CompletionTicket>,
}
struct Queue {
    generation: GenerationToken,
    resident: usize,
    worker: bool,
    pending: VecDeque<DeliveryJob>,
}
struct Activation {
    snapshot: SubscriptionSnapshot,
    running: bool,
    completion: watch::Sender<bool>,
}
impl Drop for Activation {
    fn drop(&mut self) {
        self.completion.send_replace(true);
    }
}
struct Inner {
    stats: DispatchSnapshot,
    queues: HashMap<Key, Queue>,
    activations: HashMap<Key, Activation>,
    closed: bool,
}

pub(super) struct SchedulerState {
    inner: Mutex<Inner>,
    shutdown: watch::Sender<bool>,
    activation_wake: Notify,
    ingress_slots: Arc<Semaphore>,
    attempts: Arc<Semaphore>,
    worker_slots: Arc<Semaphore>,
    activation_slots: Arc<Semaphore>,
}
impl SchedulerState {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub(super) fn snapshot(&self) -> DispatchSnapshot {
        self.lock().stats.clone()
    }
    pub(super) async fn closed(&self) {
        let mut receiver = self.shutdown.subscribe();
        while !*receiver.borrow_and_update() {
            if receiver.changed().await.is_err() {
                break;
            }
        }
    }
    fn close(&self) {
        let (queues, activations) = {
            let mut inner = self.lock();
            inner.closed = true;
            inner.stats.pending_activations = 0;
            (
                std::mem::take(&mut inner.queues),
                std::mem::take(&mut inner.activations),
            )
        };
        self.shutdown.send_replace(true);
        self.ingress_slots.close();
        self.attempts.close();
        self.worker_slots.close();
        self.activation_slots.close();
        // Job guards acquire the mutex; destroy queues after releasing it.
        drop(queues);
        drop(activations);
        self.activation_wake.notify_one();
    }
    pub(super) fn invalidate_registration(&self, tenant: &str, id: &str) {
        let k = (tenant.to_string(), id.to_string());
        let (queue, activation) = {
            let mut inner = self.lock();
            // Registration/delete has already invalidated its previous token.
            // A delayed caller must not remove a newer live generation by key.
            let queue = if inner
                .queues
                .get(&k)
                .is_some_and(|q| q.generation.is_cancelled())
            {
                inner.queues.remove(&k)
            } else {
                None
            };
            let activation = if inner
                .activations
                .get(&k)
                .is_some_and(|a| a.snapshot.generation.is_cancelled())
            {
                inner.activations.remove(&k)
            } else {
                None
            };
            if activation.as_ref().is_some_and(|a| !a.running) {
                inner.stats.pending_activations -= 1;
            }
            (queue, activation)
        };
        let removed = queue.is_some() || activation.is_some();
        drop(queue);
        drop(activation);
        if removed {
            self.activation_wake.notify_one();
        }
    }
    pub(super) fn request_activation(
        &self,
        snapshot: SubscriptionSnapshot,
    ) -> watch::Receiver<bool> {
        let k = key(&snapshot);
        let (completion, receiver) = watch::channel(false);
        let removed;
        {
            let mut inner = self.lock();
            if inner.closed || snapshot.generation.is_cancelled() {
                completion.send_replace(true);
                return receiver;
            }
            if let Some(existing) = inner
                .activations
                .get(&k)
                .filter(|a| a.snapshot.generation.same_generation(&snapshot.generation))
            {
                return existing.completion.subscribe();
            }
            removed = inner.activations.remove(&k);
            if removed.as_ref().is_some_and(|a| !a.running) {
                inner.stats.pending_activations -= 1;
            }
            inner.activations.insert(
                k,
                Activation {
                    snapshot,
                    running: false,
                    completion,
                },
            );
            inner.stats.pending_activations += 1;
        }
        drop(removed);
        self.activation_wake.notify_one();
        receiver
    }
    pub(super) fn reserve(
        self: &Arc<Self>,
        snapshot: &SubscriptionSnapshot,
    ) -> Option<ResidentGuard> {
        let k = key(snapshot);
        loop {
            let mut inner = self.lock();
            if inner.closed || snapshot.generation.is_cancelled() {
                return None;
            }
            // Direct manager registrations also invalidate their token. Retire a
            // cancelled queue before charging the replacement against global capacity.
            if inner
                .queues
                .get(&k)
                .is_some_and(|q| q.generation.is_cancelled())
            {
                let obsolete = inner.queues.remove(&k);
                drop(inner);
                drop(obsolete);
                continue;
            }
            if inner.stats.resident_notifications >= inner.stats.max_pending_notifications {
                inner.stats.dropped_global_full += 1;
                warn!(tenant_id = %k.0, subscription_id = %k.1, "Subscription notification shed: global queue full (drop-new)");
                return None;
            }
            let capacity = inner.stats.per_subscription_queue_capacity;
            let queue = inner.queues.entry(k.clone()).or_insert_with(|| Queue {
                generation: snapshot.generation.clone(),
                resident: 0,
                worker: false,
                pending: VecDeque::new(),
            });
            if !queue.generation.same_generation(&snapshot.generation) {
                return None;
            }
            if queue.resident >= capacity {
                inner.stats.dropped_subscription_full += 1;
                warn!(tenant_id = %k.0, subscription_id = %k.1, "Subscription notification shed: subscription queue full (drop-new)");
                return None;
            }
            queue.resident += 1;
            inner.stats.resident_notifications += 1;
            inner.stats.queued_notifications += 1;
            return Some(ResidentGuard {
                state: Arc::downgrade(self),
                key: k.clone(),
                generation: snapshot.generation.clone(),
                active: false,
            });
        }
    }
    pub(super) fn submit(
        self: &Arc<Self>,
        core: &Arc<EngineCore>,
        snapshot: SubscriptionSnapshot,
        bundle: serde_json::Value,
        event_number: u64,
        resident: ResidentGuard,
        completion: Option<CompletionTicket>,
    ) {
        let k = key(&snapshot);
        let generation = snapshot.generation.clone();
        let mut job = Some(DeliveryJob {
            snapshot,
            bundle,
            event_number,
            resident,
            _completion: completion,
        });
        let mut worker = None;
        {
            let mut inner = self.lock();
            if !inner.closed {
                if let Some(queue) = inner.queues.get_mut(&k).filter(|q| {
                    q.generation
                        .same_generation(&job.as_ref().unwrap().snapshot.generation)
                }) {
                    if queue.worker {
                        queue.pending.push_back(job.take().unwrap());
                    } else if let Ok(permit) = Arc::clone(&self.worker_slots).try_acquire_owned() {
                        queue.worker = true;
                        queue.pending.push_back(job.take().unwrap());
                        inner.stats.notification_workers += 1;
                        worker = Some(permit);
                    } else {
                        inner.stats.dropped_worker_full += 1;
                        warn!(tenant_id = %k.0, subscription_id = %k.1, "Subscription notification shed: worker retirement limit (drop-new)");
                    }
                }
            }
        }
        drop(job);
        if let Some(permit) = worker {
            let core = Arc::clone(core);
            let state = Arc::clone(self);
            tokio::spawn(async move {
                let _lease = WorkerLease {
                    state: Arc::clone(&state),
                    _permit: permit,
                };
                loop {
                    let Some(job) = state.next_job(&k, &generation) else {
                        break;
                    };
                    core.dispatch_with_retry(
                        &job.snapshot,
                        &job.bundle,
                        "event-notification",
                        Some(job.event_number),
                        &state,
                    )
                    .await;
                    drop(job);
                }
            });
        }
    }
    fn next_job(&self, key: &Key, generation: &GenerationToken) -> Option<DeliveryJob> {
        let mut inner = self.lock();
        let queue = inner
            .queues
            .get_mut(key)
            .filter(|q| q.generation.same_generation(generation))?;
        let job = queue.pending.pop_front();
        if let Some(mut job) = job {
            job.resident.activate(&mut inner);
            Some(job)
        } else {
            // A reservation can exist before its bundle is published. Release
            // worker ownership, retain that queue, and let submit start its worker.
            queue.worker = false;
            if queue.resident == 0 {
                inner.queues.remove(key);
            }
            None
        }
    }

    pub(super) async fn attempt(
        self: &Arc<Self>,
        snapshot: &SubscriptionSnapshot,
    ) -> Option<AttemptGuard> {
        let permit = tokio::select! {
            biased;
            _ = self.closed() => return None,
            _ = snapshot.generation.cancelled() => return None,
            permit = Arc::clone(&self.attempts).acquire_owned() => permit.ok()?,
        };
        let mut inner = self.lock();
        if inner.closed || snapshot.generation.is_cancelled() {
            return None;
        }
        inner.stats.active_attempts += 1;
        Some(AttemptGuard {
            state: Arc::clone(self),
            _permit: permit,
        })
    }
    pub(super) async fn run<F: std::future::Future>(
        &self,
        snapshot: &SubscriptionSnapshot,
        future: F,
    ) -> Option<F::Output> {
        tokio::select! { biased; _ = self.closed() => None, _ = snapshot.generation.cancelled() => None, result = future => Some(result) }
    }
}
struct WorkerLease {
    state: Arc<SchedulerState>,
    _permit: OwnedSemaphorePermit,
}
impl Drop for WorkerLease {
    fn drop(&mut self) {
        self.state.lock().stats.notification_workers -= 1;
    }
}
pub(super) struct AttemptGuard {
    state: Arc<SchedulerState>,
    _permit: OwnedSemaphorePermit,
}
impl Drop for AttemptGuard {
    fn drop(&mut self) {
        self.state.lock().stats.active_attempts -= 1;
    }
}
struct ActivationLease {
    state: Arc<SchedulerState>,
    permit: Option<OwnedSemaphorePermit>,
}
impl Drop for ActivationLease {
    fn drop(&mut self) {
        self.state.lock().stats.active_activations -= 1;
        self.permit.take();
        self.state.activation_wake.notify_one();
    }
}

pub(super) struct SchedulerOwner {
    pub(super) state: Arc<SchedulerState>,
    sender: mpsc::Sender<EventWork>,
    receiver: Mutex<Option<mpsc::Receiver<EventWork>>>,
    started: Mutex<bool>,
}
impl SchedulerOwner {
    pub(super) fn new(config: &SubscriptionConfig) -> Self {
        let (sender, receiver) = mpsc::channel(config.event_queue_capacity);
        let (shutdown, _) = watch::channel(false);
        Self {
            state: Arc::new(SchedulerState {
                inner: Mutex::new(Inner {
                    stats: DispatchSnapshot {
                        event_queue_capacity: config.event_queue_capacity,
                        max_pending_notifications: config.max_pending_notifications,
                        per_subscription_queue_capacity: config.per_subscription_queue_capacity,
                        dispatch_concurrency: config.dispatch_concurrency,
                        handshake_concurrency: config.handshake_concurrency,
                        ingress_queued: 0,
                        evaluating_events: 0,
                        resident_notifications: 0,
                        queued_notifications: 0,
                        active_notifications: 0,
                        notification_workers: 0,
                        active_attempts: 0,
                        pending_activations: 0,
                        active_activations: 0,
                        dropped_ingress_full: 0,
                        dropped_ingress_closed: 0,
                        dropped_global_full: 0,
                        dropped_subscription_full: 0,
                        dropped_worker_full: 0,
                    },
                    queues: HashMap::new(),
                    activations: HashMap::new(),
                    closed: false,
                }),
                shutdown,
                activation_wake: Notify::new(),
                ingress_slots: Arc::new(Semaphore::new(config.event_queue_capacity)),
                attempts: Arc::new(Semaphore::new(config.dispatch_concurrency)),
                worker_slots: Arc::new(Semaphore::new(config.max_pending_notifications)),
                activation_slots: Arc::new(Semaphore::new(config.handshake_concurrency)),
            }),
            sender,
            receiver: Mutex::new(Some(receiver)),
            started: Mutex::new(false),
        }
    }
    pub(super) fn start(&self, core: &Arc<EngineCore>) -> bool {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return false;
        };
        let mut started = self.started.lock().unwrap_or_else(|e| e.into_inner());
        if self.state.lock().closed {
            return false;
        }
        if *started {
            return true;
        }
        let mut receiver = self
            .receiver
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .unwrap();
        let state = Arc::clone(&self.state);
        let ingress_core = Arc::clone(core);
        runtime.spawn(async move {
            loop {
                let work = tokio::select! { biased; _ = state.closed() => break, work = receiver.recv() => work };
                let Some(work) = work else { break; };
                let EventWork { event, completion, ingress } = work;
                let _evaluating = ingress.begin_evaluation(&state);
                ingress_core.evaluate_event(&event, &state, &completion);
            }
        });
        let state = Arc::clone(&self.state);
        let core = Arc::clone(core);
        runtime.spawn(async move {
            loop {
                // Notify retains a permit when completion occurs before this wait.
                let wake = state.activation_wake.notified();
                loop {
                    let Ok(permit) = Arc::clone(&state.activation_slots).try_acquire_owned() else {
                        break;
                    };
                    let next = {
                        let mut inner = state.lock();
                        if inner.closed {
                            None
                        } else {
                            let next = inner.activations.iter_mut().find(|(_, a)| !a.running).map(
                                |(key, a)| {
                                    a.running = true;
                                    (key.clone(), a.snapshot.clone())
                                },
                            );
                            if next.is_some() {
                                inner.stats.pending_activations -= 1;
                                inner.stats.active_activations += 1;
                            }
                            next
                        }
                    };
                    let Some((k, snapshot)) = next else {
                        break;
                    };
                    let state = Arc::clone(&state);
                    let core = Arc::clone(&core);
                    tokio::spawn(async move {
                        let _lease = ActivationLease {
                            state: Arc::clone(&state),
                            permit: Some(permit),
                        };
                        if core.manager.is_current(&snapshot) && !snapshot.generation.is_cancelled()
                        {
                            core.activate_subscription(&snapshot, &state).await;
                        }
                        let removed = {
                            let mut inner = state.lock();
                            if inner.activations.get(&k).is_some_and(|a| {
                                a.snapshot.generation.same_generation(&snapshot.generation)
                            }) {
                                inner.activations.remove(&k)
                            } else {
                                None
                            }
                        };
                        drop(removed);
                    });
                }
                tokio::select! { biased; _ = state.closed() => break, _ = wake => {} }
            }
        });
        *started = true;
        true
    }
    pub(super) fn enqueue(
        &self,
        build: impl FnOnce() -> ResourceEvent,
        receipt: bool,
    ) -> Option<oneshot::Receiver<()>> {
        // The mpsc permit is released inside recv(), before the consumer can
        // update diagnostics. This independent reservation spans that handoff.
        let ingress_permit = match Arc::clone(&self.state.ingress_slots).try_acquire_owned() {
            Ok(permit) => permit,
            Err(tokio::sync::TryAcquireError::NoPermits) => {
                self.state.lock().stats.dropped_ingress_full += 1;
                warn!("Subscription resource event shed: ingress queue full (drop-new)");
                return None;
            }
            Err(tokio::sync::TryAcquireError::Closed) => {
                self.state.lock().stats.dropped_ingress_closed += 1;
                warn!("Subscription resource event shed: ingress closed");
                return None;
            }
        };
        let permit = match self.sender.try_reserve() {
            Ok(permit) => permit,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.state.lock().stats.dropped_ingress_full += 1;
                warn!("Subscription resource event shed: ingress queue full (drop-new)");
                return None;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.state.lock().stats.dropped_ingress_closed += 1;
                warn!("Subscription resource event shed: ingress closed");
                return None;
            }
        };
        {
            let mut inner = self.state.lock();
            if inner.closed {
                inner.stats.dropped_ingress_closed += 1;
                return None;
            }
            inner.stats.ingress_queued += 1;
        }
        let ingress = IngressGuard {
            state: Arc::downgrade(&self.state),
            permit: Some(ingress_permit),
        };
        let (completion, receiver) = if receipt {
            let (ticket, receiver) = CompletionTicket::new();
            (Some(ticket), Some(receiver))
        } else {
            (None, None)
        };
        permit.send(EventWork {
            event: build(),
            completion,
            ingress,
        });
        receiver
    }
}
impl Drop for SchedulerOwner {
    fn drop(&mut self) {
        self.state.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ResourceEventType;
    use crate::engine::SubscriptionEngine;
    use crate::manager::SubscriptionStatusCode;
    use crate::topics::{ResourceTrigger, TopicDefinition};
    use helios_fhir::FhirVersion;
    use helios_persistence::tenant::TenantId;
    use serde_json::json;
    use std::time::Duration;

    fn engine(limit: usize) -> SubscriptionEngine {
        let engine = SubscriptionEngine::new(
            SubscriptionConfig {
                event_queue_capacity: 1,
                max_pending_notifications: limit,
                per_subscription_queue_capacity: 1,
                dispatch_concurrency: 1,
                handshake_concurrency: 1,
                ..Default::default()
            },
            "http://localhost".into(),
        );
        engine.topic_registry().add_topic(TopicDefinition {
            canonical_url: "topic".into(),
            title: None,
            resource_triggers: vec![ResourceTrigger {
                resource_type: "Patient".into(),
                interactions: vec![ResourceEventType::Create],
                fhirpath_criteria: None,
            }],
            can_filter_by: vec![],
            notification_shape: vec![],
        });
        engine
    }
    fn snapshot(engine: &SubscriptionEngine, id: &str) -> SubscriptionSnapshot {
        let resource = crate::manager::tests::build_subscription_json(
            "topic",
            "rest-hook",
            Some("http://localhost/hook"),
        );
        engine
            .manager()
            .register("tenant", id, &resource, FhirVersion::default())
            .unwrap();
        engine
            .manager()
            .update_status("tenant", id, SubscriptionStatusCode::Active)
            .unwrap();
        engine.manager().get_snapshot("tenant", id).unwrap()
    }
    fn event() -> ResourceEvent {
        ResourceEvent {
            tenant_id: TenantId::new("tenant"),
            fhir_version: FhirVersion::default(),
            resource_type: "Patient".into(),
            resource_id: "p1".into(),
            version_id: "1".into(),
            event_type: ResourceEventType::Create,
            resource: Some(json!({"resourceType":"Patient","id":"p1"})),
            previous_resource: None,
            timestamp: chrono::Utc::now(),
        }
    }
    async fn settle(check: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !check() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("scheduler made progress");
    }
    #[tokio::test]
    async fn ingress_reserves_before_build_and_close_resolves_buffered_receipts() {
        let engine = engine(2);
        let owner = &engine.scheduler;
        let receipt = owner.enqueue(event, true).unwrap();
        assert!(
            owner
                .enqueue(|| panic!("full admission must not clone/build"), false)
                .is_none()
        );
        assert_eq!(owner.state.snapshot().dropped_ingress_full, 1);
        owner.state.close();
        drop(owner.receiver.lock().unwrap().take());
        receipt.await.unwrap();
        assert_eq!(owner.state.snapshot().ingress_queued, 0);
        assert!(
            owner
                .enqueue(|| panic!("closed admission must not build"), false)
                .is_none()
        );
        assert_eq!(owner.state.snapshot().dropped_ingress_closed, 1);
    }
    #[tokio::test]
    async fn received_event_holds_ingress_budget_until_atomic_evaluation_handoff() {
        let engine = engine(2);
        let owner = &engine.scheduler;
        let receipt = owner.enqueue(event, true).unwrap();
        let mut receiver = owner.receiver.lock().unwrap().take().unwrap();
        let work = receiver.try_recv().unwrap(); // mpsc has released its slot
        assert_eq!(owner.sender.capacity(), 1);
        assert_eq!(
            (
                owner.state.snapshot().ingress_queued,
                owner.state.snapshot().evaluating_events
            ),
            (1, 0)
        );
        assert!(
            owner
                .enqueue(
                    || panic!("received but not evaluating still owns the ingress slot"),
                    false
                )
                .is_none()
        );
        assert_eq!(owner.state.snapshot().ingress_queued, 1);
        let EventWork {
            event: first,
            completion,
            ingress,
        } = work;
        let evaluating = ingress.begin_evaluation(&owner.state);
        let snapshot = owner.state.snapshot();
        assert_eq!(
            (snapshot.ingress_queued, snapshot.evaluating_events),
            (0, 1)
        );
        let second = owner
            .enqueue(
                || {
                    let snapshot = owner.state.snapshot();
                    assert_eq!(
                        (snapshot.ingress_queued, snapshot.evaluating_events),
                        (1, 1)
                    );
                    event()
                },
                true,
            )
            .unwrap();
        drop(first);
        drop(completion);
        receipt.await.unwrap();
        drop(evaluating);
        drop(receiver.try_recv().unwrap());
        second.await.unwrap();
        let snapshot = owner.state.snapshot();
        assert_eq!(
            (snapshot.ingress_queued, snapshot.evaluating_events),
            (0, 0)
        );
        owner.state.close();
        assert!(owner.state.ingress_slots.is_closed());
        assert!(
            owner
                .enqueue(|| panic!("closed ingress must not build"), false)
                .is_none()
        );
    }

    #[test]
    fn reservations_cover_global_and_subscriber_capacity_and_failed_build_cleanup() {
        let engine = engine(2);
        let a = snapshot(&engine, "a");
        let b = snapshot(&engine, "b");
        let c = snapshot(&engine, "c");
        let state = &engine.scheduler.state;
        let first = state.reserve(&a).unwrap();
        assert!(state.reserve(&a).is_none());
        let second = state.reserve(&b).unwrap();
        assert!(state.reserve(&c).is_none());
        let view = state.snapshot();
        assert_eq!(view.resident_notifications, 2);
        assert_eq!(view.queued_notifications + view.active_notifications, 2);
        assert_eq!(
            (view.dropped_subscription_full, view.dropped_global_full),
            (1, 1)
        );
        drop(first);
        drop(second);
        assert_eq!(state.snapshot().resident_notifications, 0);
        assert!(state.lock().queues.is_empty());
    }
    #[test]
    fn retirement_preserves_unpublished_reservation_and_old_worker_cannot_remove_replacement() {
        let engine = engine(2);
        let old = snapshot(&engine, "a");
        let state = &engine.scheduler.state;
        let k = key(&old);
        let reservation = state.reserve(&old).unwrap();
        state.lock().queues.get_mut(&k).unwrap().worker = true;
        assert!(state.next_job(&k, &old.generation).is_none());
        assert_eq!(state.lock().queues.get(&k).unwrap().resident, 1);
        assert!(!state.lock().queues.get(&k).unwrap().worker);
        let new = snapshot(&engine, "a");
        state.invalidate_registration("tenant", "a");
        let replacement = state.reserve(&new).unwrap();
        assert!(state.next_job(&k, &old.generation).is_none());
        assert!(
            state
                .lock()
                .queues
                .get(&k)
                .unwrap()
                .generation
                .same_generation(&new.generation)
        );
        drop(reservation);
        drop(replacement);
        assert_eq!(state.snapshot().resident_notifications, 0);
    }
    #[tokio::test]
    async fn worker_leases_bound_retirement_and_receipts_close_after_attempt_cancellation() {
        let engine = engine(1);
        let snapshot = snapshot(&engine, "a");
        let state = &engine.scheduler.state;
        // Simulate a retiring worker: resident capacity is free but its lease remains.
        let lease = Arc::clone(&state.worker_slots)
            .acquire_owned()
            .await
            .unwrap();
        let resident = state.reserve(&snapshot).unwrap();
        let (ticket, receipt) = CompletionTicket::new();
        state.submit(
            &engine.core,
            snapshot.clone(),
            json!({}),
            1,
            resident,
            Some(ticket),
        );
        receipt.await.unwrap();
        assert_eq!(state.snapshot().dropped_worker_full, 1);
        assert_eq!(state.snapshot().resident_notifications, 0);
        drop(lease);
        // A live job waits for an attempt slot; close must cancel the wait.
        let _attempt = Arc::clone(&state.attempts).acquire_owned().await.unwrap();
        let resident = state.reserve(&snapshot).unwrap();
        let (ticket, receipt) = CompletionTicket::new();
        state.submit(&engine.core, snapshot, json!({}), 2, resident, Some(ticket));
        settle(|| state.snapshot().active_notifications == 1).await;
        state.close();
        tokio::time::timeout(Duration::from_secs(2), receipt)
            .await
            .unwrap()
            .unwrap();
        settle(|| state.snapshot().notification_workers == 0).await;
        assert_eq!(state.snapshot().resident_notifications, 0);
    }
    #[tokio::test]
    async fn aggregate_receipt_needs_no_waiting_task_and_cancellation_stops_backoff() {
        let (initial, mut receiver) = CompletionTicket::new();
        let job = initial.clone();
        drop(initial);
        assert!(matches!(
            receiver.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        drop(job);
        receiver.await.unwrap();
        let engine = engine(1);
        let snapshot = snapshot(&engine, "a");
        let sleep = engine
            .scheduler
            .state
            .run(&snapshot, tokio::time::sleep(Duration::from_secs(3600)));
        tokio::pin!(sleep);
        assert!(futures::poll!(&mut sleep).is_pending());
        engine.manager().deregister("tenant", "a");
        assert!(sleep.await.is_none());
    }
    #[tokio::test]
    async fn pending_activation_receipts_coalesce_and_replacement_does_not_clear_new_marker() {
        let engine = engine(1);
        let old = snapshot(&engine, "a");
        let state = &engine.scheduler.state;
        let mut old_receipt = state.request_activation(old.clone());
        let duplicate = state.request_activation(old.clone());
        assert_eq!(state.snapshot().pending_activations, 1);
        engine.manager().deregister("tenant", "a");
        state.invalidate_registration("tenant", "a");
        old_receipt.changed().await.unwrap();
        assert!(*old_receipt.borrow());
        assert!(*duplicate.borrow());
        let replacement = snapshot(&engine, "a");
        let mut new_receipt = state.request_activation(replacement);
        assert!(
            state
                .lock()
                .activations
                .get(&key(&old))
                .is_some_and(|a| !a.snapshot.generation.same_generation(&old.generation))
        );
        state.close();
        new_receipt.changed().await.unwrap();
        assert!(*new_receipt.borrow());
        assert_eq!(state.snapshot().pending_activations, 0);
    }
    #[tokio::test]
    async fn activation_slot_release_revisits_pending_without_another_write() {
        let mut engine = engine(2);
        Arc::get_mut(&mut engine.core)
            .unwrap()
            .config
            .handshake_initial_delay = Duration::from_millis(50);
        let a = snapshot(&engine, "a");
        let b = snapshot(&engine, "b");
        assert!(engine.scheduler.start(&engine.core));
        let mut first = engine.scheduler.state.request_activation(a);
        settle(|| engine.dispatch_snapshot().active_activations == 1).await;
        let mut second = engine.scheduler.state.request_activation(b);
        assert_eq!(engine.dispatch_snapshot().pending_activations, 1);
        tokio::time::timeout(Duration::from_secs(2), async {
            while !*first.borrow_and_update() {
                first.changed().await.unwrap();
            }
            while !*second.borrow_and_update() {
                second.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        settle(|| engine.dispatch_snapshot().active_activations == 0).await;
        assert_eq!(engine.dispatch_snapshot().pending_activations, 0);
    }

    #[tokio::test]
    async fn attempt_waiters_acquire_fairly_and_release_the_slot_between_attempts() {
        let engine = engine(2);
        let a = snapshot(&engine, "a");
        let b = snapshot(&engine, "b");
        let state = &engine.scheduler.state;
        let held = Arc::clone(&state.attempts).acquire_owned().await.unwrap();
        let first = state.attempt(&a);
        let second = state.attempt(&b);
        tokio::pin!(first, second);
        assert!(futures::poll!(&mut first).is_pending());
        assert!(futures::poll!(&mut second).is_pending());
        drop(held);
        let first = first.await.unwrap();
        assert_eq!(state.snapshot().active_attempts, 1);
        assert!(futures::poll!(&mut second).is_pending());
        drop(first);
        let second = second.await.unwrap();
        assert_eq!(state.snapshot().active_attempts, 1);
        drop(second);
        assert_eq!(state.snapshot().active_attempts, 0);
    }

    #[tokio::test]
    async fn late_lifecycle_cleanup_preserves_live_replacement_queue_and_activation() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let hook = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&hook)
            .await;
        let engine = engine(2);
        let old = snapshot(&engine, "a");
        let old_reservation = engine.scheduler.state.reserve(&old).unwrap();
        // The new registration invalidates A; B publishes its own work before
        // A's lifecycle caller reaches its cleanup step.
        let resource =
            crate::manager::tests::build_subscription_json("topic", "rest-hook", Some(&hook.uri()));
        let replacement = engine
            .manager()
            .register_snapshot("tenant", "a", &resource, FhirVersion::default(), Some("2"))
            .unwrap();
        let state = &engine.scheduler.state;
        let resident = state
            .reserve(&replacement)
            .expect("obsolete queue cleaned during reservation");
        let (ticket, mut delivery_receipt) = CompletionTicket::new();
        {
            let mut inner = state.lock();
            let queue = inner.queues.get_mut(&key(&replacement)).unwrap();
            queue.worker = true;
            queue.pending.push_back(DeliveryJob {
                snapshot: replacement.clone(),
                bundle: json!({}),
                event_number: 1,
                resident,
                _completion: Some(ticket),
            });
        }
        let mut activation_receipt = state.request_activation(replacement.clone());
        state.invalidate_registration("tenant", "a"); // delayed cleanup from A
        assert!(matches!(
            delivery_receipt.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert!(!*activation_receipt.borrow());
        assert!(state.lock().queues.contains_key(&key(&replacement)));
        assert_eq!(state.snapshot().pending_activations, 1);
        assert!(engine.scheduler.start(&engine.core));
        tokio::time::timeout(Duration::from_secs(2), async {
            while !*activation_receipt.borrow_and_update() {
                activation_receipt.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(
            engine
                .manager()
                .get_subscription("tenant", "a")
                .unwrap()
                .status,
            SubscriptionStatusCode::Active
        );
        let job = state
            .next_job(&key(&replacement), &replacement.generation)
            .unwrap();
        drop(job);
        delivery_receipt.await.unwrap();
        drop(old_reservation);
        assert_eq!(state.snapshot().resident_notifications, 0);
    }

    #[test]
    fn late_delete_cleanup_preserves_a_live_recreation() {
        let engine = engine(2);
        let old = snapshot(&engine, "a");
        let state = &engine.scheduler.state;
        let old_resident = state.reserve(&old).unwrap();
        engine.manager().deregister("tenant", "a");
        let recreation = snapshot(&engine, "a");
        let new_resident = state.reserve(&recreation).unwrap();
        let receipt = state.request_activation(recreation.clone());
        state.invalidate_registration("tenant", "a");
        assert!(
            state
                .lock()
                .queues
                .get(&key(&recreation))
                .unwrap()
                .generation
                .same_generation(&recreation.generation)
        );
        assert!(!*receipt.borrow());
        drop(old_resident);
        drop(new_resident);
        state.close();
    }

    #[tokio::test]
    async fn engine_drop_does_not_leave_a_task_ownership_cycle() {
        let engine = engine(1);
        let weak = Arc::downgrade(&engine.core);
        assert!(engine.scheduler.start(&engine.core));
        drop(engine);
        settle(|| weak.upgrade().is_none()).await;
    }
}
