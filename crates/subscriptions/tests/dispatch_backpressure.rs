//! Public-entry-point regressions for bounded dispatch (#2018).
//! Receivers hold real HTTP responses; scheduler snapshots count only engine work.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::core::{
    ResourceStorage, ResourceWrite, WriteEvent, WriteKind, WriteNotice, WriteObserver,
};
use helios_persistence::error::StorageResult;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::StoredResource;
use helios_subscriptions::manager::SubscriptionStatusCode;
use helios_subscriptions::{
    DispatchSnapshot, RehydrationConfig, ResourceEvent, ResourceEventType, SubscriptionConfig,
    SubscriptionEngine, SubscriptionWriteObserver,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};
use tokio::task::{JoinHandle, JoinSet};

const TOPIC: &str = "https://example.org/topic/dispatch-backpressure";
const WAIT: Duration = Duration::from_secs(5);
static NEXT_TENANT: AtomicUsize = AtomicUsize::new(0);

fn version() -> FhirVersion {
    match std::env::var("HFS_TEST_FHIR_VERSION").ok().as_deref() {
        #[cfg(feature = "R4B")]
        Some("R4B") => FhirVersion::R4B,
        #[cfg(feature = "R5")]
        Some("R5") => FhirVersion::R5,
        #[cfg(feature = "R6")]
        Some("R6") => FhirVersion::R6,
        _ => FhirVersion::default(),
    }
}

fn topic_resource(id: &str, canonical: &str) -> Value {
    if version() == FhirVersion::R4 {
        json!({"resourceType":"Basic", "id":id,
            "code":{"coding":[{"system":"http://hl7.org/fhir/fhir-types","code":"SubscriptionTopic"}]},
            "extension":[
                {"url":"http://hl7.org/fhir/5.0/StructureDefinition/extension-SubscriptionTopic.url","valueUri":canonical},
                {"url":"http://hl7.org/fhir/4.3/StructureDefinition/extension-SubscriptionTopic.resourceTrigger","extension":[
                    {"url":"resource","valueUri":"http://hl7.org/fhir/StructureDefinition/Encounter"},
                    {"url":"supportedInteraction","valueCode":"create"}]}]})
    } else {
        json!({"resourceType":"SubscriptionTopic","id":id,"url":canonical,"status":"active",
            "resourceTrigger":[{"resource":"Encounter","supportedInteraction":["create"]}]})
    }
}

fn subscription_resource(id: &str, endpoint: &str, status: &str) -> Value {
    if version() == FhirVersion::R4 {
        json!({"resourceType":"Subscription","id":id,"status":status,"criteria":TOPIC,
            "channel":{"type":"rest-hook","endpoint":endpoint,"payload":"application/fhir+json"}})
    } else {
        json!({"resourceType":"Subscription","id":id,"status":status,"topic":TOPIC,
            "channelType":{"code":"rest-hook"},"endpoint":endpoint,"content":"id-only",
            "contentType":"application/fhir+json"})
    }
}

fn event(tenant: &str, resource: Value) -> ResourceEvent {
    ResourceEvent {
        tenant_id: TenantId::new(tenant),
        fhir_version: version(),
        resource_type: resource["resourceType"].as_str().unwrap().into(),
        resource_id: resource["id"].as_str().unwrap().into(),
        version_id: "1".into(),
        event_type: ResourceEventType::Create,
        resource: Some(resource),
        previous_resource: None,
        timestamp: Utc::now(),
    }
}

fn deletion(tenant: &str, resource_type: &str, id: &str) -> ResourceEvent {
    let mut event = event(tenant, json!({"resourceType":resource_type,"id":id}));
    event.event_type = ResourceEventType::Delete;
    event.resource = None;
    event
}

fn tenant_context(tenant: &str) -> TenantContext {
    TenantContext::new(TenantId::new(tenant), TenantPermissions::full_access())
}

fn limits() -> SubscriptionConfig {
    SubscriptionConfig {
        event_queue_capacity: 32,
        max_pending_notifications: 32,
        per_subscription_queue_capacity: 8,
        dispatch_concurrency: 4,
        handshake_concurrency: 2,
        max_retries: 1,
        retry_initial_delay: Duration::from_millis(10),
        retry_max_delay: Duration::from_millis(20),
        error_threshold: 1_000,
        off_threshold: 1_000,
        ..SubscriptionConfig::default()
    }
}

async fn wait_until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(WAIT, async {
        loop {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("condition reached before timeout");
}

async fn done(task: JoinHandle<()>) {
    tokio::time::timeout(WAIT, task)
        .await
        .expect("completion receipt resolved")
        .expect("event task did not panic");
}

fn assert_bounded(snapshot: &DispatchSnapshot) {
    assert!(
        snapshot.ingress_queued <= snapshot.event_queue_capacity,
        "{snapshot:?}"
    );
    assert!(snapshot.evaluating_events <= 1, "{snapshot:?}");
    assert!(
        snapshot.resident_notifications <= snapshot.max_pending_notifications,
        "{snapshot:?}"
    );
    assert!(
        snapshot.notification_workers <= snapshot.max_pending_notifications,
        "{snapshot:?}"
    );
    assert!(
        snapshot.active_attempts <= snapshot.dispatch_concurrency,
        "{snapshot:?}"
    );
    assert!(
        snapshot.active_activations <= snapshot.handshake_concurrency,
        "{snapshot:?}"
    );
}

#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<String>>>);
impl tracing::Subscriber for Warnings {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() == tracing::Level::WARN
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        struct Message(String);
        impl tracing::field::Visit for Message {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.0 = format!("{value:?}");
                }
            }
        }
        let mut message = Message(String::new());
        event.record(&mut message);
        self.0.lock().unwrap().push(message.0);
    }
}

#[derive(Debug, Clone)]
struct Notice {
    kind: String,
    number: Option<u64>,
    focus: Option<String>,
}

fn notice(bundle: &Value) -> Notice {
    let status = &bundle["entry"][0]["resource"];
    if status["resourceType"] == "Parameters" {
        let parameters = status["parameter"].as_array().unwrap();
        let value = |name| parameters.iter().find(|p| p["name"] == name);
        let notification = value("notification-event").and_then(|p| p["part"].as_array());
        let part = |name| notification.and_then(|parts| parts.iter().find(|p| p["name"] == name));
        Notice {
            kind: value("type").unwrap()["valueCode"].as_str().unwrap().into(),
            number: part("event-number")
                .and_then(|p| p["valueString"].as_str())
                .and_then(|v| v.parse().ok()),
            focus: part("focus")
                .and_then(|p| p["valueReference"]["reference"].as_str())
                .map(str::to_owned),
        }
    } else {
        Notice {
            kind: status["type"].as_str().unwrap().into(),
            number: status["notificationEvent"][0]["eventNumber"].as_u64(),
            focus: status["notificationEvent"][0]["focus"]["reference"]
                .as_str()
                .map(str::to_owned),
        }
    }
}

struct HeldRequest {
    notice: Notice,
    response: oneshot::Sender<u16>,
}
impl HeldRequest {
    fn reply(self, status: u16) {
        let _ = self.response.send(status);
    }
}

struct ReceiverState {
    hold_events: AtomicBool,
    hold_handshakes: AtomicBool,
    records: Mutex<Vec<Notice>>,
    held: mpsc::UnboundedSender<HeldRequest>,
}

/// Each response belongs to its request, so release cannot race a later request.
struct GatedReceiver {
    endpoint: String,
    state: Arc<ReceiverState>,
    held: AsyncMutex<mpsc::UnboundedReceiver<HeldRequest>>,
    server: JoinHandle<()>,
}
impl GatedReceiver {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/notify", listener.local_addr().unwrap());
        let (sender, receiver) = mpsc::unbounded_channel();
        let state = Arc::new(ReceiverState {
            hold_events: AtomicBool::new(true),
            hold_handshakes: AtomicBool::new(false),
            records: Mutex::new(Vec::new()),
            held: sender,
        });
        let server_state = Arc::clone(&state);
        let server = tokio::spawn(async move {
            let mut handlers = JoinSet::new();
            loop {
                tokio::select! {
                    incoming = listener.accept() => {
                        let Ok((socket, _)) = incoming else { break; };
                        handlers.spawn(receive(socket, Arc::clone(&server_state)));
                    }
                    finished = handlers.join_next(), if !handlers.is_empty() => {
                        if let Some(Err(error)) = finished { assert!(error.is_cancelled(), "receiver handler: {error}"); }
                    }
                }
            }
        });
        Self {
            endpoint,
            state,
            held: AsyncMutex::new(receiver),
            server,
        }
    }
    fn auto_events(&self) {
        self.state.hold_events.store(false, Ordering::SeqCst);
    }
    fn hold_handshakes(&self) {
        self.state.hold_handshakes.store(true, Ordering::SeqCst);
    }
    fn records(&self, kind: &str) -> Vec<Notice> {
        self.state
            .records
            .lock()
            .unwrap()
            .iter()
            .filter(|n| n.kind == kind)
            .cloned()
            .collect()
    }
    fn events(&self) -> Vec<Notice> {
        self.records("event-notification")
    }
    async fn next(&self) -> HeldRequest {
        tokio::time::timeout(WAIT, self.held.lock().await.recv())
            .await
            .expect("HTTP request reached receiver")
            .expect("receiver open")
    }
}
impl Drop for GatedReceiver {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn receive(mut socket: TcpStream, state: Arc<ReceiverState>) {
    let mut bytes = Vec::new();
    let mut chunk = [0; 4096];
    let (body_start, length) = loop {
        let Ok(size) = socket.read(&mut chunk).await else {
            return;
        };
        if size == 0 {
            return;
        }
        bytes.extend_from_slice(&chunk[..size]);
        assert!(bytes.len() < 2 * 1024 * 1024, "fixture request too large");
        if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            let header = std::str::from_utf8(&bytes[..end]).unwrap();
            let length = header
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .expect("JSON POST has content length");
            break (end + 4, length);
        }
    };
    while bytes.len() < body_start + length {
        let Ok(size) = socket.read(&mut chunk).await else {
            return;
        };
        if size == 0 {
            return;
        }
        bytes.extend_from_slice(&chunk[..size]);
    }
    let bundle: Value = serde_json::from_slice(&bytes[body_start..body_start + length]).unwrap();
    let notice = notice(&bundle);
    state.records.lock().unwrap().push(notice.clone());
    let held = match notice.kind.as_str() {
        "handshake" => state.hold_handshakes.load(Ordering::SeqCst),
        "event-notification" => state.hold_events.load(Ordering::SeqCst),
        _ => false,
    };
    let status = if held {
        let (sender, receiver) = oneshot::channel();
        if state
            .held
            .send(HeldRequest {
                notice,
                response: sender,
            })
            .is_err()
        {
            return;
        }
        tokio::select! {
            response = receiver => match response { Ok(status) => status, Err(_) => return },
            // Cancelled requests can close before the test releases their response.
            _ = socket.read(&mut chunk) => return,
        }
    } else {
        200
    };
    let response =
        format!("HTTP/1.1 {status} Fixture\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _ = socket.write_all(response.as_bytes()).await;
    let _ = socket.shutdown().await;
}

struct Fixture {
    engine: Arc<SubscriptionEngine>,
    tenant: String,
}
impl Fixture {
    async fn new(config: SubscriptionConfig) -> Self {
        let tenant = format!(
            "dispatch-{}-{}",
            std::process::id(),
            NEXT_TENANT.fetch_add(1, Ordering::SeqCst)
        );
        let engine = Arc::new(SubscriptionEngine::new(
            config,
            "https://example.org/fhir".into(),
        ));
        let fixture = Self { engine, tenant };
        fixture
            .engine
            .on_resource_event(event(&fixture.tenant, topic_resource("topic", TOPIC)))
            .await;
        fixture
    }
    async fn add(&self, id: &str, receiver: &GatedReceiver) {
        self.engine
            .on_resource_event(event(
                &self.tenant,
                subscription_resource(id, &receiver.endpoint, "requested"),
            ))
            .await;
        assert_eq!(self.status(id), SubscriptionStatusCode::Active);
    }
    fn status(&self, id: &str) -> SubscriptionStatusCode {
        self.engine
            .manager()
            .get_subscription(&self.tenant, id)
            .unwrap()
            .status
    }
    fn encounter(&self, id: &str) -> ResourceEvent {
        event(
            &self.tenant,
            json!({"resourceType":"Encounter","id":id,"status":"in-progress"}),
        )
    }
    fn spawn(&self, event: ResourceEvent) -> JoinHandle<()> {
        let engine = Arc::clone(&self.engine);
        tokio::spawn(async move {
            engine.on_resource_event(event).await;
        })
    }
    fn write(&self, event: ResourceEvent) {
        write(&self.engine, event);
    }
    async fn delete(&self, id: &str) {
        self.engine
            .on_resource_event(deletion(&self.tenant, "Subscription", id))
            .await;
    }
    fn snapshot(&self) -> DispatchSnapshot {
        let snapshot = self.engine.dispatch_snapshot();
        assert_bounded(&snapshot);
        snapshot
    }
    async fn idle(&self) {
        wait_until(|| {
            let s = self.snapshot();
            s.ingress_queued == 0
                && s.evaluating_events == 0
                && s.resident_notifications == 0
                && s.notification_workers == 0
                && s.active_attempts == 0
                && s.active_activations == 0
                && s.pending_activations == 0
        })
        .await;
    }
}

fn write(engine: &Arc<SubscriptionEngine>, event: ResourceEvent) {
    let kind = match event.event_type {
        ResourceEventType::Create => WriteKind::Create,
        ResourceEventType::Update => WriteKind::Update,
        ResourceEventType::Delete => WriteKind::Delete,
    };
    SubscriptionWriteObserver::new(Arc::clone(engine)).on_write(&WriteEvent::Resource(
        ResourceWrite {
            tenant: event.tenant_id,
            fhir_version: event.fhir_version,
            resource_type: event.resource_type,
            live_delta: 1,
            notice: Some(WriteNotice {
                kind,
                resource_id: event.resource_id,
                version_id: event.version_id,
                resource: event.resource,
                previous: event.previous_resource,
            }),
            at: event.timestamp,
        },
    ));
}

async fn healthy_independence(hanging: bool, attempts: usize) {
    let fixture = Fixture::new(SubscriptionConfig {
        dispatch_concurrency: attempts,
        max_retries: 2,
        retry_initial_delay: Duration::from_secs(10),
        retry_max_delay: Duration::from_secs(10),
        ..limits()
    })
    .await;
    let mut receivers = Vec::new();
    for index in 0..4 {
        let receiver = GatedReceiver::new().await;
        let id = format!("sub-{index}");
        fixture.add(&id, &receiver).await;
        receivers.push((id, receiver));
    }
    // Match order is stable without registrations. Make its first receiver bad,
    // so the old sequential implementation cannot accidentally pass this test.
    let first = fixture
        .engine
        .manager()
        .active_subscriptions_for_topic(&fixture.tenant, TOPIC)[0]
        .id
        .clone();
    let bad = receivers.iter().position(|(id, _)| id == &first).unwrap();
    for (index, (_, receiver)) in receivers.iter().enumerate() {
        if index != bad {
            receiver.auto_events();
        }
    }
    let task = fixture.spawn(fixture.encounter("healthy-independence"));
    let held = receivers[bad].1.next().await;
    assert_eq!(held.notice.kind, "event-notification");
    let hanging_request = if hanging {
        Some(held)
    } else {
        held.reply(503);
        None
    };
    wait_until(|| {
        receivers
            .iter()
            .enumerate()
            .all(|(index, (_, receiver))| index == bad || receiver.events().len() == 1)
    })
    .await;
    assert!(
        !task.is_finished(),
        "healthy delivery must precede the failing cycle's completion"
    );
    assert_eq!(receivers[bad].1.events().len(), 1);
    if hanging {
        assert_eq!(fixture.snapshot().active_attempts, 1);
    } else {
        wait_until(|| fixture.snapshot().active_attempts == 0).await;
    }
    fixture.delete(&first).await;
    if let Some(held) = hanging_request {
        held.reply(200);
    }
    done(task).await;
    fixture.idle().await;
}

#[tokio::test]
async fn receiver_503_does_not_delay_healthy_same_topic() {
    healthy_independence(false, 4).await;
}

#[tokio::test]
async fn hanging_receiver_does_not_delay_healthy_same_topic() {
    healthy_independence(true, 4).await;
}

#[tokio::test]
async fn retry_backoff_releases_attempt_permit() {
    let fixture = Fixture::new(SubscriptionConfig {
        dispatch_concurrency: 1,
        per_subscription_queue_capacity: 1,
        max_retries: 2,
        retry_initial_delay: Duration::from_secs(10),
        retry_max_delay: Duration::from_secs(10),
        ..limits()
    })
    .await;
    let bad = GatedReceiver::new().await;
    let healthy = GatedReceiver::new().await;
    healthy.auto_events();
    fixture.add("bad", &bad).await;
    let first = fixture.spawn(fixture.encounter("bad-first"));
    let held = bad.next().await;
    // Register an already-active subscriber while bad owns the only attempt.
    fixture
        .engine
        .on_resource_event(event(
            &fixture.tenant,
            subscription_resource("healthy", &healthy.endpoint, "active"),
        ))
        .await;
    let second = fixture.spawn(fixture.encounter("healthy-after-bad"));
    wait_until(|| fixture.snapshot().resident_notifications == 2).await;
    assert!(healthy.events().is_empty());
    assert_eq!(fixture.snapshot().active_attempts, 1);
    held.reply(503);
    wait_until(|| healthy.events().len() == 1).await;
    done(second).await;
    assert!(!first.is_finished(), "bad is still in backoff");
    assert_eq!(bad.events().len(), 1);
    fixture.delete("bad").await;
    done(first).await;
    fixture.idle().await;
}

#[tokio::test]
async fn observer_ingress_sheds_before_spawning_and_recovers_under_down_receiver() {
    let fixture = Fixture::new(SubscriptionConfig {
        event_queue_capacity: 2,
        max_pending_notifications: 3,
        per_subscription_queue_capacity: 3,
        dispatch_concurrency: 1,
        ..limits()
    })
    .await;
    let receiver = GatedReceiver::new().await;
    fixture.add("bad", &receiver).await;
    let warnings = Warnings::default();
    for round in 0..3 {
        tracing::subscriber::with_default(warnings.clone(), || {
            for index in 0..10 {
                fixture.write(fixture.encounter(&format!("round-{round}-{index}")));
            }
        });
        let snapshot = fixture.snapshot();
        assert_eq!(snapshot.ingress_queued, 2);
        assert_eq!(snapshot.dropped_ingress_full, 8 * (round + 1));
        for _ in 0..2 {
            let held = receiver.next().await;
            assert_bounded(&fixture.snapshot());
            held.reply(503);
        }
        fixture.idle().await;
    }
    let window =
        fixture
            .engine
            .delivery_stats()
            .window(&fixture.tenant, "bad", Utc::now().timestamp());
    assert_eq!(
        window.failed, 6,
        "shed ingress must not count as receiver failures"
    );
    let subscription = fixture
        .engine
        .manager()
        .get_subscription(&fixture.tenant, "bad")
        .unwrap();
    assert_eq!(
        subscription.events_since_start, 6,
        "pre-evaluation shedding consumes no number"
    );
    assert_eq!(subscription.consecutive_failures, 6);
    assert_eq!(receiver.events().len(), 6);
    assert_eq!(
        warnings
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|message| message
                .contains("Subscription resource event shed: ingress queue full (drop-new)"))
            .count(),
        24
    );
}

#[tokio::test]
async fn direct_events_share_ingress_bound_and_resolve_dropped_receipts() {
    let fixture = Fixture::new(SubscriptionConfig {
        event_queue_capacity: 2,
        ..limits()
    })
    .await;
    let receiver = GatedReceiver::new().await;
    fixture.add("sub", &receiver).await;
    let mut pending: Vec<Pin<Box<dyn Future<Output = ()> + '_>>> = Vec::new();
    let mut context = Context::from_waker(Waker::noop());
    for index in 0..5 {
        let mut future = Box::pin(
            fixture
                .engine
                .on_resource_event(fixture.encounter(&format!("direct-{index}"))),
        );
        match future.as_mut().poll(&mut context) {
            Poll::Pending => pending.push(future),
            Poll::Ready(()) => assert!(index >= 2, "only overflow receipts resolve immediately"),
        }
    }
    assert_eq!(pending.len(), 2);
    assert_eq!(fixture.snapshot().ingress_queued, 2);
    assert_eq!(fixture.snapshot().dropped_ingress_full, 3);
    receiver.next().await.reply(200);
    receiver.next().await.reply(200);
    tokio::time::timeout(WAIT, futures::future::join_all(pending))
        .await
        .expect("accepted receipts resolved");
    fixture.idle().await;
    assert_eq!(receiver.events().len(), 2);
}

#[tokio::test]
async fn global_budget_bounds_fanout_and_counts_drop_new() {
    let fixture = Fixture::new(SubscriptionConfig {
        max_pending_notifications: 2,
        per_subscription_queue_capacity: 2,
        dispatch_concurrency: 2,
        ..limits()
    })
    .await;
    let mut receivers = Vec::new();
    for index in 0..3 {
        let receiver = GatedReceiver::new().await;
        fixture.add(&format!("sub-{index}"), &receiver).await;
        receivers.push(receiver);
    }
    let task = fixture.spawn(fixture.encounter("fanout"));
    wait_until(|| {
        fixture.snapshot().resident_notifications == 2
            && fixture.snapshot().dropped_global_full == 1
    })
    .await;
    wait_until(|| receivers.iter().map(|r| r.events().len()).sum::<usize>() == 2).await;
    assert_eq!(fixture.snapshot().active_attempts, 2);
    for receiver in &receivers {
        if !receiver.events().is_empty() {
            receiver.next().await.reply(200);
        }
    }
    done(task).await;
    fixture.idle().await;
    for index in 0..3 {
        let id = format!("sub-{index}");
        assert_eq!(
            fixture
                .engine
                .manager()
                .get_subscription(&fixture.tenant, &id)
                .unwrap()
                .events_since_start,
            1
        );
        assert_eq!(
            fixture
                .engine
                .delivery_stats()
                .window(&fixture.tenant, &id, Utc::now().timestamp())
                .failed,
            0
        );
    }
}

#[tokio::test]
async fn per_subscription_overflow_preserves_fifo_number_gaps_and_capacity() {
    let fixture = Fixture::new(SubscriptionConfig {
        per_subscription_queue_capacity: 2,
        ..limits()
    })
    .await;
    let receiver = GatedReceiver::new().await;
    fixture.add("sub", &receiver).await;
    let first = fixture.spawn(fixture.encounter("one"));
    let held = receiver.next().await;
    fixture.write(fixture.encounter("two"));
    wait_until(|| fixture.snapshot().resident_notifications == 2).await;
    fixture.write(fixture.encounter("three-shed"));
    wait_until(|| fixture.snapshot().dropped_subscription_full == 1).await;
    assert_eq!(receiver.events().len(), 1);
    assert_eq!(
        fixture
            .engine
            .manager()
            .get_subscription(&fixture.tenant, "sub")
            .unwrap()
            .consecutive_failures,
        0
    );
    held.reply(200);
    let second = receiver.next().await;
    assert_eq!(second.notice.number, Some(2));
    second.reply(200);
    done(first).await;
    fixture.idle().await;
    let fourth = fixture.spawn(fixture.encounter("four"));
    receiver.next().await.reply(200);
    done(fourth).await;
    fixture.idle().await;
    let events = receiver.events();
    assert_eq!(
        events.iter().map(|n| n.number.unwrap()).collect::<Vec<_>>(),
        [1, 2, 4]
    );
    assert_eq!(
        events
            .iter()
            .map(|n| n.focus.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["Encounter/one", "Encounter/two", "Encounter/four"]
    );
}

#[tokio::test]
async fn retry_sleeps_retain_global_budget_and_preserve_fifo() {
    let fixture = Fixture::new(SubscriptionConfig {
        max_pending_notifications: 2,
        per_subscription_queue_capacity: 2,
        max_retries: 2,
        retry_initial_delay: Duration::from_millis(100),
        retry_max_delay: Duration::from_millis(200),
        ..limits()
    })
    .await;
    let receiver = GatedReceiver::new().await;
    fixture.add("sub", &receiver).await;
    let first = fixture.spawn(fixture.encounter("one"));
    let held = receiver.next().await;
    fixture.write(fixture.encounter("two"));
    wait_until(|| fixture.snapshot().resident_notifications == 2).await;
    held.reply(503);
    wait_until(|| fixture.snapshot().active_attempts == 0).await;
    assert_eq!(fixture.snapshot().resident_notifications, 2);
    fixture.write(fixture.encounter("three-shed"));
    wait_until(|| fixture.snapshot().dropped_global_full == 1).await;
    let retry = receiver.next().await;
    assert_eq!(retry.notice.number, Some(1));
    assert_eq!(
        receiver
            .events()
            .iter()
            .map(|n| n.number.unwrap())
            .collect::<Vec<_>>(),
        [1, 1]
    );
    retry.reply(200);
    let second = receiver.next().await;
    assert_eq!(second.notice.number, Some(2));
    second.reply(200);
    done(first).await;
    fixture.idle().await;
}

#[tokio::test]
async fn same_subscription_id_across_tenants_has_independent_queues() {
    let fixture = Fixture::new(SubscriptionConfig {
        per_subscription_queue_capacity: 1,
        ..limits()
    })
    .await;
    let bad = GatedReceiver::new().await;
    let healthy = GatedReceiver::new().await;
    healthy.auto_events();
    fixture.add("shared-id", &bad).await;
    let other = format!("{}-other", fixture.tenant);
    fixture
        .engine
        .on_resource_event(event(
            &other,
            subscription_resource("shared-id", &healthy.endpoint, "requested"),
        ))
        .await;
    let bad_task = fixture.spawn(fixture.encounter("bad"));
    let held = bad.next().await;
    fixture.write(fixture.encounter("bad-overflow"));
    wait_until(|| fixture.snapshot().dropped_subscription_full == 1).await;
    fixture
        .engine
        .on_resource_event(event(
            &other,
            json!({"resourceType":"Encounter","id":"healthy"}),
        ))
        .await;
    assert_eq!(healthy.events().len(), 1);
    assert_eq!(healthy.events()[0].number, Some(1));
    assert_eq!(bad.events().len(), 1);
    fixture.delete("shared-id").await;
    held.reply(200);
    done(bad_task).await;
    assert_eq!(
        fixture
            .engine
            .manager()
            .get_subscription(&other, "shared-id")
            .unwrap()
            .status,
        SubscriptionStatusCode::Active
    );
    fixture.idle().await;
}

#[tokio::test]
async fn control_writes_bypass_full_data_ingress_including_basic_delete_paths() {
    let fixture = Fixture::new(SubscriptionConfig {
        event_queue_capacity: 2,
        ..limits()
    })
    .await;
    fixture.write(fixture.encounter("one"));
    fixture.write(fixture.encounter("two"));
    assert_eq!(fixture.snapshot().ingress_queued, 2);
    let canonical = "https://example.org/topic/overload-control";
    let topic = topic_resource("control", canonical);
    let resource_type = topic["resourceType"].as_str().unwrap().to_owned();
    fixture.write(event(&fixture.tenant, topic));
    assert!(
        fixture
            .engine
            .topic_registry()
            .get_topic(canonical)
            .is_some()
    );
    fixture.write(event(
        &fixture.tenant,
        subscription_resource("control", "https://example.org/unused", "off"),
    ));
    assert_eq!(fixture.status("control"), SubscriptionStatusCode::Off);
    fixture.write(deletion(&fixture.tenant, "Subscription", "control"));
    assert!(
        fixture
            .engine
            .manager()
            .get_subscription(&fixture.tenant, "control")
            .is_none()
    );
    fixture.write(deletion(&fixture.tenant, &resource_type, "control"));
    assert!(
        fixture
            .engine
            .topic_registry()
            .get_topic(canonical)
            .is_none()
    );
    if version() == FhirVersion::R4 {
        let previous_url = "https://example.org/topic/previous-only";
        let previous = topic_resource("previous-only", previous_url);
        let topic = helios_subscriptions::topics::InMemoryTopicRegistry::parse_r4_backport_basic_topic_resource(&previous).unwrap().unwrap();
        fixture.engine.topic_registry().add_topic(topic);
        let mut deleted = deletion(&fixture.tenant, "Basic", "previous-only");
        deleted.previous_resource = Some(previous);
        fixture.write(deleted);
        assert!(
            fixture
                .engine
                .topic_registry()
                .get_topic(previous_url)
                .is_none()
        );
    }
    fixture.write(event(
        &fixture.tenant,
        json!({"resourceType":"Basic","id":"ordinary","code":{"text":"ordinary data"}}),
    ));
    assert_eq!(
        fixture.snapshot().dropped_ingress_full,
        1,
        "ordinary Basic remains data ingress"
    );
    assert_eq!(
        fixture.snapshot().ingress_queued,
        2,
        "control traffic did not occupy data queue"
    );
    fixture.idle().await;
}

#[tokio::test]
async fn ordinary_update_cancels_old_jobs_and_late_results_with_capacity_recovery() {
    let fixture = Fixture::new(SubscriptionConfig {
        max_pending_notifications: 2,
        per_subscription_queue_capacity: 2,
        ..limits()
    })
    .await;
    let old = GatedReceiver::new().await;
    let new = GatedReceiver::new().await;
    fixture.add("sub", &old).await;
    let task = fixture.spawn(fixture.encounter("old-one"));
    let held = old.next().await;
    fixture.write(fixture.encounter("old-two"));
    wait_until(|| fixture.snapshot().resident_notifications == 2).await;
    let mut updated = event(
        &fixture.tenant,
        subscription_resource("sub", &new.endpoint, "active"),
    );
    updated.event_type = ResourceEventType::Update;
    fixture.engine.on_resource_event(updated).await;
    done(task).await;
    fixture.idle().await;
    held.reply(503); // An already-sent request cannot mutate its replacement.
    let task = fixture.spawn(fixture.encounter("new-three"));
    let held = new.next().await;
    assert_eq!(held.notice.number, Some(3));
    held.reply(200);
    done(task).await;
    fixture.idle().await;
    assert_eq!(
        old.events().len(),
        1,
        "queued old endpoint work was cancelled"
    );
    let subscription = fixture
        .engine
        .manager()
        .get_subscription(&fixture.tenant, "sub")
        .unwrap();
    assert_eq!(subscription.events_since_start, 3);
    assert_eq!(subscription.consecutive_failures, 0);
    assert_eq!(
        fixture
            .engine
            .delivery_stats()
            .window(&fixture.tenant, "sub", Utc::now().timestamp())
            .failed,
        0
    );
}

#[tokio::test]
async fn delete_recreate_discards_old_generation_and_resolves_receipts() {
    let fixture = Fixture::new(limits()).await;
    let old = GatedReceiver::new().await;
    let new = GatedReceiver::new().await;
    fixture.add("sub", &old).await;
    let task = fixture.spawn(fixture.encounter("old-one"));
    let held = old.next().await;
    fixture.write(fixture.encounter("old-two"));
    wait_until(|| fixture.snapshot().resident_notifications == 2).await;
    fixture.delete("sub").await;
    done(task).await;
    fixture.idle().await;
    fixture.add("sub", &new).await;
    held.reply(503);
    let task = fixture.spawn(fixture.encounter("new-one"));
    let held = new.next().await;
    assert_eq!(held.notice.number, Some(1));
    held.reply(200);
    done(task).await;
    fixture.idle().await;
    assert_eq!(old.events().len(), 1);
    assert_eq!(fixture.status("sub"), SubscriptionStatusCode::Active);
    assert_eq!(
        fixture
            .engine
            .manager()
            .get_subscription(&fixture.tenant, "sub")
            .unwrap()
            .consecutive_failures,
        0
    );
}

#[tokio::test]
async fn error_drains_admitted_until_off_cancels_remainder_then_rearm_resets() {
    let fixture = Fixture::new(SubscriptionConfig {
        max_pending_notifications: 4,
        per_subscription_queue_capacity: 4,
        error_threshold: 1,
        off_threshold: 3,
        ..limits()
    })
    .await;
    let old = GatedReceiver::new().await;
    let new = GatedReceiver::new().await;
    fixture.add("sub", &old).await;
    let first = fixture.spawn(fixture.encounter("one"));
    let held = old.next().await;
    for id in ["two", "three", "four-cancelled"] {
        fixture.write(fixture.encounter(id));
    }
    wait_until(|| fixture.snapshot().resident_notifications == 4).await;
    held.reply(503);
    let second = old.next().await;
    assert_eq!(fixture.status("sub"), SubscriptionStatusCode::Error);
    fixture
        .engine
        .on_resource_event(fixture.encounter("not-matched-in-error"))
        .await;
    assert_eq!(
        fixture
            .engine
            .manager()
            .get_subscription(&fixture.tenant, "sub")
            .unwrap()
            .events_since_start,
        4
    );
    second.reply(503);
    let third = old.next().await;
    assert_eq!(third.notice.number, Some(3));
    third.reply(503);
    done(first).await;
    fixture.idle().await;
    assert_eq!(fixture.status("sub"), SubscriptionStatusCode::Off);
    assert_eq!(old.events().len(), 3);
    let mut rearm = event(
        &fixture.tenant,
        subscription_resource("sub", &new.endpoint, "requested"),
    );
    rearm.event_type = ResourceEventType::Update;
    fixture.engine.on_resource_event(rearm).await;
    assert_eq!(fixture.status("sub"), SubscriptionStatusCode::Active);
    let task = fixture.spawn(fixture.encounter("rearmed-one"));
    let held = new.next().await;
    assert_eq!(held.notice.number, Some(1));
    held.reply(200);
    done(task).await;
    fixture.idle().await;
    assert_eq!(
        fixture
            .engine
            .manager()
            .get_subscription(&fixture.tenant, "sub")
            .unwrap()
            .consecutive_failures,
        0
    );
}

struct ReadGate {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}
struct GatedStore {
    inner: Arc<SqliteBackend>,
    gate: Mutex<Option<ReadGate>>,
    updates: AtomicUsize,
}
impl GatedStore {
    fn new() -> Arc<Self> {
        let inner = SqliteBackend::in_memory().unwrap();
        inner.init_schema().unwrap();
        Arc::new(Self {
            inner: Arc::new(inner),
            gate: Mutex::new(None),
            updates: AtomicUsize::new(0),
        })
    }
    fn hold_next_read(&self) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (entered, received) = oneshot::channel();
        let (release, released) = oneshot::channel();
        assert!(
            self.gate
                .lock()
                .unwrap()
                .replace(ReadGate {
                    entered,
                    release: released
                })
                .is_none()
        );
        (received, release)
    }
}
#[async_trait]
impl ResourceStorage for GatedStore {
    fn backend_name(&self) -> &'static str {
        "gated-sqlite"
    }
    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        self.inner
            .create(tenant, resource_type, resource, version)
            .await
    }
    async fn create_or_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
        version: FhirVersion,
    ) -> StorageResult<(StoredResource, bool)> {
        self.inner
            .create_or_update(tenant, resource_type, id, resource, version)
            .await
    }
    async fn read(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let gate = self.gate.lock().unwrap().take();
        if let Some(gate) = gate {
            let _ = gate.entered.send(());
            let _ = gate.release.await;
        }
        // The client may have committed a newer intent while this read waited.
        self.inner.read(tenant, resource_type, id).await
    }
    async fn update(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        self.updates.fetch_add(1, Ordering::SeqCst);
        self.inner.update(tenant, current, resource).await
    }
    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        self.inner.delete(tenant, resource_type, id).await
    }
    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        self.inner.count(tenant, resource_type).await
    }
}

async fn stale_status_read(reregister: bool) {
    let mut fixture = Fixture::new(limits()).await;
    let store = GatedStore::new();
    fixture.engine = Arc::new(
        Arc::try_unwrap(fixture.engine)
            .ok()
            .unwrap()
            .with_status_store(store.clone()),
    );
    let old = GatedReceiver::new().await;
    let new = GatedReceiver::new().await;
    new.hold_handshakes();
    let tenant = tenant_context(&fixture.tenant);
    let stored = store
        .inner
        .create(
            &tenant,
            "Subscription",
            subscription_resource("sub", &old.endpoint, "requested"),
            version(),
        )
        .await
        .unwrap();
    let (entered, release) = store.hold_next_read();
    let mut registered = event(&fixture.tenant, stored.content().clone());
    registered.version_id = stored.version_id().into();
    let old_task = fixture.spawn(registered);
    tokio::time::timeout(WAIT, entered).await.unwrap().unwrap();
    let mut client_content = subscription_resource("sub", &new.endpoint, "requested");
    client_content["reason"] = json!("new client intent");
    let client = store
        .inner
        .update(&tenant, &stored, client_content)
        .await
        .unwrap();
    let new_task = if reregister {
        let mut update = event(&fixture.tenant, client.content().clone());
        update.event_type = ResourceEventType::Update;
        update.version_id = client.version_id().into();
        Some(fixture.spawn(update))
    } else {
        None
    };
    let held = if reregister {
        Some(new.next().await)
    } else {
        None
    };
    let _ = release.send(());
    done(old_task).await;
    let current = store
        .inner
        .read(&tenant, "Subscription", "sub")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.version_id(), client.version_id());
    assert_eq!(
        current.content()["status"],
        "requested",
        "old write must not overwrite newer stored intent"
    );
    assert_eq!(current.content()["reason"], "new client intent");
    assert_eq!(
        store.updates.load(Ordering::SeqCst),
        0,
        "stale generation or source fence rejects before update"
    );
    if let Some(held) = held {
        assert_eq!(held.notice.kind, "handshake");
        held.reply(200);
        done(new_task.unwrap()).await;
        let current = store
            .inner
            .read(&tenant, "Subscription", "sub")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.content()["status"], "active");
        assert_eq!(current.content()["reason"], "new client intent");
        assert_eq!(store.updates.load(Ordering::SeqCst), 1);
    }
    fixture.idle().await;
}

#[tokio::test]
async fn late_status_read_cannot_write_after_generation_replacement() {
    stale_status_read(true).await;
}

#[tokio::test]
async fn late_status_read_cannot_write_newer_storage_before_observer_catches_up() {
    stale_status_read(false).await;
}

fn sqlite() -> Arc<SqliteBackend> {
    let backend = SqliteBackend::in_memory().unwrap();
    backend.init_schema().unwrap();
    Arc::new(backend)
}

async fn seed(backend: &SqliteBackend, tenant: &str, resource: Value) {
    backend
        .create(
            &tenant_context(tenant),
            resource["resourceType"].as_str().unwrap(),
            resource.clone(),
            version(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn startup_and_live_handshakes_share_cap_and_coalesce_pending_latest_activation() {
    let fixture = Fixture::new(SubscriptionConfig {
        handshake_concurrency: 1,
        ..limits()
    })
    .await;
    let backend = sqlite();
    let startup = GatedReceiver::new().await;
    let obsolete = GatedReceiver::new().await;
    let latest = GatedReceiver::new().await;
    startup.hold_handshakes();
    obsolete.hold_handshakes();
    latest.hold_handshakes();
    seed(&backend, &fixture.tenant, topic_resource("topic", TOPIC)).await;
    seed(
        &backend,
        &fixture.tenant,
        subscription_resource("startup", &startup.endpoint, "requested"),
    )
    .await;
    let engine = Arc::clone(&fixture.engine);
    let storage = Arc::clone(&backend);
    let tenant = fixture.tenant.clone();
    let rehydrate = tokio::spawn(async move {
        let report = engine
            .rehydrate(
                storage.as_ref(),
                &tenant,
                version(),
                &RehydrationConfig::default(),
            )
            .await;
        assert!(report.is_complete(), "{report:?}");
        assert_eq!(report.handshakes_started, 1);
    });
    let held = startup.next().await;
    fixture.write(event(
        &fixture.tenant,
        subscription_resource("live", &obsolete.endpoint, "requested"),
    ));
    for _ in 0..10 {
        let mut update = event(
            &fixture.tenant,
            subscription_resource("live", &latest.endpoint, "requested"),
        );
        update.event_type = ResourceEventType::Update;
        fixture.write(update);
    }
    let snapshot = fixture.snapshot();
    assert_eq!(snapshot.active_activations, 1);
    assert_eq!(
        snapshot.pending_activations, 1,
        "one pending latest generation, no per-write activations"
    );
    assert!(obsolete.records("handshake").is_empty());
    assert!(latest.records("handshake").is_empty());
    held.reply(200);
    let held = latest.next().await;
    assert_eq!(held.notice.kind, "handshake");
    assert_eq!(fixture.snapshot().active_activations, 1);
    held.reply(200);
    done(rehydrate).await;
    fixture.idle().await;
    assert_eq!(fixture.status("startup"), SubscriptionStatusCode::Active);
    assert_eq!(fixture.status("live"), SubscriptionStatusCode::Active);
    assert!(obsolete.records("handshake").is_empty());
    assert_eq!(latest.records("handshake").len(), 1);
}

#[tokio::test]
async fn handshake_and_notification_attempts_share_dispatch_limit() {
    let fixture = Fixture::new(SubscriptionConfig {
        dispatch_concurrency: 1,
        ..limits()
    })
    .await;
    let event_receiver = GatedReceiver::new().await;
    let activation_receiver = GatedReceiver::new().await;
    activation_receiver.hold_handshakes();
    fixture.add("active", &event_receiver).await;
    let task = fixture.spawn(fixture.encounter("held-event"));
    let held = event_receiver.next().await;
    fixture.write(event(
        &fixture.tenant,
        subscription_resource("requested", &activation_receiver.endpoint, "requested"),
    ));
    wait_until(|| fixture.snapshot().active_activations == 1).await;
    assert_eq!(fixture.snapshot().active_attempts, 1);
    assert!(activation_receiver.records("handshake").is_empty());
    held.reply(200);
    let held = activation_receiver.next().await;
    assert_eq!(fixture.snapshot().active_attempts, 1);
    held.reply(200);
    done(task).await;
    fixture.idle().await;
    assert_eq!(fixture.status("requested"), SubscriptionStatusCode::Active);
}

#[tokio::test]
async fn disabled_rehydration_and_disabled_startup_handshakes_make_no_outbound_requests() {
    let fixture = Fixture::new(limits()).await;
    let backend = sqlite();
    let receiver = GatedReceiver::new().await;
    receiver.hold_handshakes();
    seed(&backend, &fixture.tenant, topic_resource("topic", TOPIC)).await;
    seed(
        &backend,
        &fixture.tenant,
        subscription_resource("requested", &receiver.endpoint, "requested"),
    )
    .await;
    let disabled = fixture
        .engine
        .rehydrate(
            backend.as_ref(),
            &fixture.tenant,
            version(),
            &RehydrationConfig {
                enabled: false,
                ..RehydrationConfig::default()
            },
        )
        .await;
    assert_eq!(disabled.subscriptions_registered, 0);
    assert!(
        fixture
            .engine
            .manager()
            .get_subscription(&fixture.tenant, "requested")
            .is_none()
    );
    let dormant = fixture
        .engine
        .rehydrate(
            backend.as_ref(),
            &fixture.tenant,
            version(),
            &RehydrationConfig {
                handshake_requested: false,
                ..RehydrationConfig::default()
            },
        )
        .await;
    assert_eq!(dormant.subscriptions_registered, 1);
    assert_eq!(dormant.handshakes_started, 0);
    assert_eq!(
        fixture.status("requested"),
        SubscriptionStatusCode::Requested
    );
    fixture.idle().await;
    assert!(receiver.records("handshake").is_empty());
}

#[tokio::test]
async fn aborted_caller_keeps_accepted_work_until_delete_then_owner_drop_closes_io() {
    let fixture = Fixture::new(limits()).await;
    let receiver = GatedReceiver::new().await;
    fixture.add("sub", &receiver).await;
    let task = fixture.spawn(fixture.encounter("caller-aborted"));
    let held = receiver.next().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(fixture.snapshot().resident_notifications, 1);
    fixture.delete("sub").await;
    fixture.idle().await;
    held.reply(200);
    fixture.add("sub", &receiver).await;
    fixture.write(fixture.encounter("owner-dropped"));
    let mut held = receiver.next().await;
    let weak = Arc::downgrade(&fixture.engine);
    drop(fixture);
    assert!(
        weak.upgrade().is_none(),
        "background dispatch owns core, not an engine ownership cycle"
    );
    tokio::time::timeout(WAIT, held.response.closed())
        .await
        .expect("owner drop cancels request and receiver observes EOF");
}

#[test]
fn invalid_constructor_limits_normalize_without_a_runtime() {
    let defaults = SubscriptionConfig::default();
    let engine = SubscriptionEngine::new(
        SubscriptionConfig {
            event_queue_capacity: 0,
            max_pending_notifications: usize::MAX,
            per_subscription_queue_capacity: 0,
            dispatch_concurrency: usize::MAX,
            handshake_concurrency: 0,
            ..limits()
        },
        "https://example.org/fhir".into(),
    );
    let snapshot = engine.dispatch_snapshot();
    assert_eq!(snapshot.event_queue_capacity, defaults.event_queue_capacity);
    assert_eq!(
        snapshot.max_pending_notifications,
        defaults.max_pending_notifications
    );
    assert_eq!(
        snapshot.per_subscription_queue_capacity,
        defaults.per_subscription_queue_capacity
    );
    assert_eq!(snapshot.dispatch_concurrency, defaults.dispatch_concurrency);
    assert_eq!(
        snapshot.handshake_concurrency,
        defaults.handshake_concurrency
    );
    assert_eq!(snapshot.active_attempts, 0);
    assert_eq!(snapshot.notification_workers, 0);
}
