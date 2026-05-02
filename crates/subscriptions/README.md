# helios-subscriptions

FHIR topic-based Subscriptions engine for the Helios FHIR Server, implementing the [FHIR Subscriptions Framework](https://build.fhir.org/subscriptions.html) across R4, R4B, R5, and R6.

R4 uses the [Subscriptions R5 Backport IG](https://build.fhir.org/ig/HL7/fhir-subscription-backport-ig/). R4B, R5, and R6 use native subscription resources.

## Overview

This crate implements topic-based subscriptions as an asynchronous pipeline that fires after every resource create, update, or delete. These are the three interaction types defined by the FHIR [`SubscriptionTopic.resourceTrigger.supportedInteraction`](https://hl7.org/fhir/subscriptiontopic.html) value set. It decomposes into five concerns:

1. **Topic Registry** — stores `SubscriptionTopic` definitions and evaluates resource triggers
2. **Subscription Manager** — tracks active `Subscription` resources and their runtime state
3. **Event Evaluator** — matches write events against active subscriptions using topic triggers and filter criteria
4. **Notification Builder** — constructs version-specific notification bundles (R4 Parameters-based backport, R4B/R5/R6 native `SubscriptionStatus`)
5. **Channel Dispatcher** — delivers notifications via pluggable channel implementations

The `SubscriptionEngine` orchestrates all five concerns and is the main entry point, invoked via `tokio::spawn` after each resource create, update, patch, or delete — mirroring the fire-and-forget pattern used by the audit middleware.

## Features

- **Version-aware parsing**: R4 reads the backport extension set (`backport-topic-canonical`, `backport-payload-content`, `backport-channel-type`, `backport-filter-criteria`); R4B/R5/R6 read native `topic`, `channelType`, `filterBy` fields
- **Topic lifecycle parity**: topic create/update/delete now updates the in-memory registry for both native `SubscriptionTopic` and R4 backport `Basic` topic resources
- **Status state machine**: `Requested → Active → Error → Off` with validated transitions; subscriptions activate only after a successful handshake
- **Exponential backoff retry**: configurable initial delay, max delay, backoff factor, and max attempts before transitioning to `error` or `off`
- **Tenant isolation**: all in-memory maps are keyed by `(tenant_id, subscription_id)` — subscriptions in different tenants never interact
- **TLS enforcement**: `full-resource` payload subscriptions over non-HTTPS endpoints are rejected at dispatch time
- **WebSocket channel**: server-managed connection registry with short-lived binding tokens; clients connect to `/ws/subscriptions/bind`, then send `bind-with-token <token>` after calling `$get-ws-binding-token`
- **Pluggable channels**: `ChannelDispatcher` trait allows new channel types (email, FHIR messaging) to be added without touching the engine

## Channel Support

| Channel | Status | Notes |
|---------|--------|-------|
| `rest-hook` | Implemented | HTTP POST with custom headers, TLS enforcement for full-resource payloads |
| `websocket` | Implemented | Binding token flow, per-subscription client registry, unidirectional server→client streaming |
| `email` | Implemented | SMTP via `lettre`; plain-text body + `notification.json` attachment for FHIR JSON payloads; encrypted SMTP (STARTTLS/TLS) required for `full-resource` payloads |
| `fhir-messaging` | Planned (Phase 4) | Notification wrapped in a FHIR message Bundle |

## Architecture

```
ResourceEvent
     │
     ▼
SubscriptionEngine.on_resource_event()
     │
     ├─ resource_type == "Subscription"   → SubscriptionManager.register() / deregister()
     ├─ resource_type == "SubscriptionTopic" → add/update/remove topic in InMemoryTopicRegistry
     ├─ resource_type == "Basic" && fhir_version == "R4"
     │    → parse as R4 backport topic candidate; add/remove topic when valid
     │
     └─ otherwise:
           │
           ▼
       EventEvaluator.evaluate()
           │  finds matching topics + subscriptions + applies filter criteria
           ▼
       NotificationBundleBuilder.build()
           │  R4: Bundle(history) + Parameters-based SubscriptionStatus
           │  R4B/R5/R6: Bundle + native SubscriptionStatus
           ▼
       ChannelDispatcher.dispatch()
           │  rest-hook: HTTP POST with retry
           │  websocket: broadcast to connected clients (best-effort)
           ▼
       handle_delivery_failure() on exhaustion
           │  consecutive_failures >= error_threshold → Error
           │  consecutive_failures >= off_threshold   → Off
```

## Notification Bundle Format

### R4 (Backport IG)

```json
{
  "resourceType": "Bundle",
  "type": "history",
  "entry": [
    {
      "resource": {
        "resourceType": "Parameters",
        "parameter": [
          { "name": "subscription", "valueReference": { "reference": "Subscription/sub-1" } },
          { "name": "topic", "valueCanonical": "http://example.org/topic/encounter-start" },
          { "name": "status", "valueCode": "active" },
          { "name": "type", "valueCode": "event-notification" },
          { "name": "events-since-subscription-start", "valueString": "3" },
          {
            "name": "notification-event",
            "part": [
              { "name": "event-number", "valueString": "3" },
              { "name": "timestamp", "valueInstant": "2026-04-09T12:00:00Z" },
              { "name": "focus", "valueReference": { "reference": "Encounter/enc-99" } }
            ]
          }
        ]
      }
    }
  ]
}
```

### R4B/R5/R6 (Native)

`Bundle.type` is `history` for R4B and `subscription-notification` for R5/R6.

```json
{
  "resourceType": "Bundle",
  "type": "<history|subscription-notification>",
  "entry": [
    {
      "resource": {
        "resourceType": "SubscriptionStatus",
        "status": "active",
        "type": "event-notification",
        "eventsSinceSubscriptionStart": 3,
        "subscription": { "reference": "Subscription/sub-1" },
        "topic": "http://example.org/topic/encounter-start",
        "notificationEvent": [
          {
            "eventNumber": 3,
            "timestamp": "2026-04-09T12:00:00Z",
            "focus": { "reference": "Encounter/enc-99" }
          }
        ]
      }
    }
  ]
}
```

## Filter Matching

Filters use the R4 backport string format `ResourceType?parameter=value` (parsed by `parse_filter_string`) or the native R4B/R5/R6 `filterBy` array. The evaluator supports:

| Filter parameter | Resolved from |
|-----------------|---------------|
| `code` | `CodeableConcept.coding[].code` tokens |
| `category` | `CodeableConcept.coding[].code` tokens |
| `patient` / `subject` | `subject.reference` or `patient.reference` |
| `identifier` | `identifier[].value` |
| *(other)* | Direct JSON field lookup by name |

Comparators supported: `eq` (default), `in`. Native `filterBy.comparator` values are mapped into the internal filter form. FHIRPath evaluation is not used in Phase 2 — filters currently operate on the raw resource JSON.

## Configuration

`SubscriptionConfig` is constructed programmatically or from environment variables when running inside HFS:

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_SUBSCRIPTIONS_ENABLED` | `false` | Enable the subscription engine |
| `HFS_SUBSCRIPTION_MAX_RETRIES` | `10` | Max delivery attempts before marking error |
| `HFS_SUBSCRIPTION_RETRY_INITIAL_DELAY` | `1s` | Initial delay for exponential backoff |
| `HFS_SUBSCRIPTION_RETRY_MAX_DELAY` | `60s` | Maximum delay cap for backoff |
| `HFS_SUBSCRIPTION_HANDSHAKE_INITIAL_DELAY_MS` | `0` | Delay before the first activation handshake attempt |
| `HFS_SUBSCRIPTION_HANDSHAKE_MAX_ATTEMPTS` | `1` | Max activation handshake attempts before marking error |
| `HFS_SUBSCRIPTION_HANDSHAKE_RETRY_BASE_MS` | `1000` | Initial delay before retrying a failed activation handshake |
| `HFS_SUBSCRIPTION_HANDSHAKE_RETRY_MAX_MS` | `60000` | Maximum delay cap for activation handshake retries |
| `HFS_SUBSCRIPTION_HEARTBEAT_INTERVAL` | `30s` | How often to check for due heartbeats |
| `HFS_SUBSCRIPTION_ERROR_THRESHOLD` | `3` | Consecutive failures before `error` status |
| `HFS_SUBSCRIPTION_OFF_THRESHOLD` | `10` | Consecutive failures before `off` status |
| `ws_token_lifetime_secs` *(config field)* | `30` | Binding token expiry in seconds (WebSocket only) |

### Email channel (SMTP)

The email channel is enabled only when `HFS_SUBSCRIPTION_SMTP_HOST` and `HFS_SUBSCRIPTION_SMTP_FROM` are both set. With either missing, `email` is omitted from the server's supported channel list and any email `Subscription` is rejected with an `unsupported channel type` OperationOutcome.

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_SUBSCRIPTION_SMTP_HOST` | (unset → email disabled) | SMTP relay host. |
| `HFS_SUBSCRIPTION_SMTP_PORT` | `25` | SMTP relay port. |
| `HFS_SUBSCRIPTION_SMTP_USERNAME` | (none) | SMTP auth username. |
| `HFS_SUBSCRIPTION_SMTP_PASSWORD` | (none) | SMTP auth password. |
| `HFS_SUBSCRIPTION_SMTP_ENCRYPTION` | `starttls` | `none` \| `starttls` \| `tls`. |
| `HFS_SUBSCRIPTION_SMTP_FROM` | (unset → email disabled) | Default `From:` mailbox (RFC 5322). |
| `HFS_SUBSCRIPTION_SMTP_DEFAULT_SUBJECT` | built-in template | Default `Subject:` template. Supports `{notification-type}`, `{topic-url}`, `{event-number}`. |
| `HFS_SUBSCRIPTION_SMTP_TIMEOUT_SECS` | `30` | Per-send timeout. |

Subscription `header` entries (R4 backport `channel.header`, native `Subscription.parameter`) named `Subject`, `From`, `Reply-To`, and `Cc` override the server defaults on a per-subscription basis. Subscriptions requesting `content=full-resource` over an `HFS_SUBSCRIPTION_SMTP_ENCRYPTION=none` transport are rejected at dispatch time (analogous to the HTTPS requirement on the rest-hook channel).

### FHIR Messaging channel

The FHIR Messaging channel is disabled by default and is enabled only when `HFS_SUBSCRIPTION_MESSAGING_ENABLED=true`.

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_SUBSCRIPTION_MESSAGING_ENABLED` | `false` | Enable the FHIR Messaging subscription channel. |
| `HFS_SUBSCRIPTION_MESSAGE_SOURCE_ENDPOINT` | `HFS_BASE_URL` | Source endpoint URL used in outbound FHIR message headers. |
| `HFS_SUBSCRIPTION_ALLOW_PRIVATE_ENDPOINTS` | `false` | Allow delivery to private or loopback endpoint hosts; intended for local development and CI only. |

## Enabling in HFS

The subscription engine is an optional feature in `helios-rest` and `helios-hfs`:

```bash
# Build with subscriptions support
cargo build --bin hfs --features subscriptions

# Enable at runtime
HFS_SUBSCRIPTIONS_ENABLED=true cargo run --bin hfs --features subscriptions
```

For version-specific validation, use explicit feature selection:

```bash
cargo check -p helios-subscriptions --no-default-features --features R4
cargo check -p helios-subscriptions --no-default-features --features R4B
cargo check -p helios-subscriptions --no-default-features --features R5
cargo check -p helios-subscriptions --no-default-features --features R6
```

External smoke script supports version selection through `FHIR_VERSION` (`R4`, `R4B`, `R5`, `R6`).

When enabled, the engine auto-initializes with default configuration and begins processing events after the first resource write.

## Integration

The engine integrates into `helios-rest` via the `AppState::with_subscription_engine()` builder. After each successful write, handlers call `emit_subscription_event()`. Topic lifecycle writes are processed inline so the topic registry is up to date before the HTTP response returns; subscription activation and ordinary resource notifications are dispatched asynchronously:

```rust,ignore
// In create handler (simplified)
if let Some(engine) = state.subscription_engine() {
    emit_subscription_event(engine, tenant.context(), &stored, fhir_version, ResourceEventType::Create).await;
}
```

For asynchronous events, the spawned task calls `engine.on_resource_event(event).await`, which runs the full evaluation -> notification -> dispatch pipeline without blocking the HTTP response.

### Registering a Topic

POST a `SubscriptionTopic` resource (R4B/R5/R6) or a `Basic` resource with the backport profile (R4) to your HFS instance. The engine picks it up automatically on the next write:

**R4B/R5/R6 (native `SubscriptionTopic`)**

```bash
curl -X POST http://localhost:8080/SubscriptionTopic \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "SubscriptionTopic",
    "url": "http://example.org/topic/encounter-start",
    "status": "active",
    "resourceTrigger": [{
      "resource": "Encounter",
      "supportedInteraction": ["create", "update"]
    }],
    "canFilterBy": [{
      "resource": "Encounter",
      "filterParameter": "patient"
    }]
  }'
```

**R4 (backport `Basic` topic representation)**

```bash
curl -X POST http://localhost:8080/Basic \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "Basic",
    "code": {
      "coding": [{
        "system": "http://hl7.org/fhir/fhir-types",
        "code": "SubscriptionTopic"
      }]
    },
    "extension": [{
      "url": "http://hl7.org/fhir/5.0/StructureDefinition/extension-SubscriptionTopic.url",
      "valueUri": "http://example.org/topic/encounter-start"
    }, {
      "url": "http://hl7.org/fhir/4.3/StructureDefinition/extension-SubscriptionTopic.resourceTrigger",
      "extension": [{
        "url": "resource",
        "valueUri": "http://hl7.org/fhir/StructureDefinition/Encounter"
      }, {
        "url": "supportedInteraction",
        "valueCode": "create"
      }]
    }]
  }'
```

### Creating a Subscription

```bash
curl -X POST http://localhost:8080/Subscription \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "Subscription",
    "status": "requested",
    "topic": "http://example.org/topic/encounter-start",
    "channelType": { "code": "rest-hook" },
    "endpoint": "https://your-server.example.com/webhook",
    "content": "id-only",
    "filterBy": [{
      "filterParameter": "patient",
      "value": "Patient/123"
    }]
  }'
```

The server schedules an activation handshake notification to the endpoint. On a successful 2xx response the subscription transitions to `active`; retryable handshake failures can be retried according to the `HFS_SUBSCRIPTION_HANDSHAKE_*` settings.

### Creating a WebSocket Subscription

```bash
curl -X POST http://localhost:8080/Subscription \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "Subscription",
    "status": "requested",
    "topic": "http://example.org/topic/encounter-start",
    "channelType": { "code": "websocket" },
    "content": "full-resource"
  }'
```

WebSocket subscriptions activate immediately (no outbound handshake is required). The endpoint field is not used — the server provides the WebSocket URL via the binding token operation.

### Creating an Email Subscription

With the server configured per the SMTP env vars above:

```bash
curl -X POST http://localhost:8080/Subscription \
  -H "Content-Type: application/fhir+json" \
  -d '{
    "resourceType": "Subscription",
    "status": "requested",
    "topic": "http://example.org/topic/encounter-start",
    "channelType": { "code": "email" },
    "endpoint": "mailto:nurse@example.com",
    "contentType": "application/fhir+json",
    "content": "id-only",
    "parameter": [
      { "name": "Subject", "value": "Encounter started" },
      { "name": "Reply-To", "value": "ops@example.com" }
    ]
  }'
```

The handshake notification is delivered as an email whose body is a short human-readable summary and whose `notification.json` attachment carries the full FHIR notification Bundle. Subsequent event notifications follow the same shape.

### Connecting via WebSocket

After creating a WebSocket subscription, clients connect using a short-lived binding token:

**Step 1 — Get a binding token**

```bash
GET /Subscription/{id}/$get-ws-binding-token
```

Returns a `Parameters` resource:

```json
{
  "resourceType": "Parameters",
  "parameter": [
    { "name": "token",         "valueString": "550e8400-e29b-41d4-a716-446655440000" },
    { "name": "expiration",    "valueDateTime": "2026-04-12T10:00:30Z" },
    { "name": "websocket-url", "valueUrl": "ws://localhost:8080/ws/subscriptions/bind" }
  ]
}
```

Tokens are single-use and expire after 30 seconds (configurable via `ws_token_lifetime_secs`).

**Step 2 — Connect**

```bash
# Using websocat
websocat ws://localhost:8080/ws/subscriptions/bind
# Then send:
bind-with-token 550e8400-e29b-41d4-a716-446655440000
```

After a successful bind message, the server sends a handshake notification bundle. Subsequent event notifications are pushed as JSON text frames as matching resources are written to the server.

The WebSocket protocol is **unidirectional** (server → client) after binding. Additional bind messages on the same connection are rejected; close frames trigger graceful cleanup.

**Multiple clients:** Multiple clients can bind to the same subscription simultaneously. All connected clients receive every notification.

**Subscription deletion:** Deleting the `Subscription` resource closes all connected WebSocket clients for that subscription.

### Checking Subscription Status

```bash
GET /Subscription/{id}/$status
```

Returns a `Parameters` resource (R4) or `SubscriptionStatus` resource (R4B/R5/R6) with the current runtime status and event count.

## Features

FHIR version support via Cargo feature flags:

| Feature | Default | Description |
|---------|---------|-------------|
| `R4` | Yes | FHIR R4 with Subscriptions R5 Backport IG |
| `R4B` | No | FHIR R4B with native Subscriptions resources |
| `R5` | No | FHIR R5 with native Subscriptions Framework |
| `R6` | No | FHIR R6 with native Subscriptions Framework |

```toml
[dependencies]
helios-subscriptions = { version = "0.1", features = ["R4"] }
```

## Design Considerations

### Performance: `tokio::spawn` per event

The current design spawns one Tokio task per resource write. Each task runs the full evaluate → build → dispatch pipeline.

**Where this works well:**

- Task creation is cheap — a dozen allocations, no syscall. Thousands per second is not a problem for Tokio's scheduler.
- In-memory evaluation is fast — topic matching and filter checks on `DashMap` are microseconds.
- Low-volume servers handling tens of writes per second are well-served by this model.

**Where it breaks down:**

- **Unbounded concurrency on HTTP dispatch.** If an endpoint is slow or down, tasks accumulate — each holding memory for the request, the notification bundle, and sleeping during retry backoff. A burst of 1,000 writes to a subscription with a failing endpoint spawns 1,000 tasks, each retrying up to 10 times.
- **No ordering guarantees.** Tokio tasks are scheduled cooperatively — event 2 can be dispatched before event 1. The engine assigns monotonic `eventNumber`s, but HTTP POSTs can arrive out of order at the receiver.
- **Retry holds tasks alive.** A single dispatch with max retries and a 60s backoff cap can keep a task alive for ~2 minutes (1+2+4+8+16+32+60+60+60+60s), holding its full context in memory the entire time.
- **No event coalescing.** If a resource is updated 5 times in rapid succession, 5 separate tasks run the full pipeline. No deduplication or batching.
- **Fan-out is sequential.** If 50 subscriptions match an event, `dispatch_with_retry` runs sequentially for each within a single task. One slow endpoint blocks all subsequent dispatches in that task.

### Alternative dispatch approaches (future phases)

| Approach | Description | Trade-off |
|----------|-------------|-----------|
| **Semaphore on spawn** | Add a `tokio::sync::Semaphore` to cap concurrent outbound HTTP calls (e.g., 32). Minimal change. | Prevents unbounded fan-out but doesn't solve ordering or retry memory. |
| **Bounded channel + worker pool** | Replace `tokio::spawn` with `tokio::sync::mpsc`. N worker tasks pull events from the channel. Backpressure is built in. | Standard production pattern. Solves backpressure and enables graceful shutdown. Recommended next step. |
| **Per-subscription queues** | One queue per `(tenant, subscription_id)`. Events to the same subscription are strictly ordered; a slow endpoint only blocks its own queue. | "Actor per subscription" model. Good for ordering guarantees, heavier to implement. |
| **Deferred retry queue** | Failed deliveries go to a separate retry queue with a scheduled wake-up, freeing the worker immediately. | Pairs well with the channel + worker pool approach. Eliminates long-lived sleeping tasks. |

### Clustering

The current architecture is **single-instance only**. The `InMemoryTopicRegistry` and `SubscriptionManager` are process-local `DashMap`s with no shared state between instances.

In a clustered deployment behind a load balancer:

```
                    Load Balancer
                   /             \
              Instance A         Instance B
              ┌──────────┐      ┌──────────┐
              │ Topics: 1│      │ Topics: 0│
              │ Subs:   3│      │ Subs:   0│
              └──────────┘      └──────────┘

POST /SubscriptionTopic → routed to A → only A has the topic
POST /Subscription      → routed to A → only A tracks it
POST /Encounter         → routed to B → B has no topics → no notification fires
```

Additional problems in a multi-instance deployment:

- **Duplicate notifications.** If events are somehow visible to multiple instances, each fires independently — the subscriber receives the same notification N times.
- **Split event counters.** `eventNumber` and `events_since_subscription_start` are per-instance counters. Instance A says event #5, Instance B says event #3 — the subscriber sees non-monotonic, duplicated sequence numbers.
- **Split failure tracking.** Instance A records 2 consecutive failures, Instance B records 1. Neither reaches the `error_threshold` of 3, so the subscription never transitions to `error` even though the endpoint has failed 3 times total.
- **Handshake races.** Two instances could both try to activate the same subscription simultaneously, sending duplicate handshake notifications.

**Common production approaches:**

| Approach | Description |
|----------|-------------|
| **Leader election** | One instance is elected (via distributed lock / lease) as the subscription processor. Others publish events to a shared queue (Postgres NOTIFY/LISTEN, Redis streams, Kafka). Simple, avoids duplication, but the leader is a bottleneck / SPOF. |
| **Subscription partitioning** | Subscriptions are hash-partitioned across instances (`subscription_id % N`). Each instance only dispatches for its assigned subscriptions. Rebalances on scale-up/down. |
| **Shared DB state / outbox pattern** | Topics, subscriptions, event counters, and failure counts live in the database. Events are written as outbox rows in the same transaction as the resource write. A single consumer dispatches from the outbox. |
| **DB change streams** | The engine subscribes to the database's change feed (Postgres logical replication, MongoDB change streams) rather than hooking into REST handlers. Any write — including batch/transaction — triggers evaluation. |

### Kafka-Based Event Bus (Future Phase)

A Kafka-backed architecture addresses most of the single-instance and performance limitations simultaneously. In this model, REST handlers publish a lightweight `ResourceEvent` to a Kafka topic instead of spawning a local task, and a separate consumer group evaluates and dispatches notifications:

```
  HFS Instance A ──┐                          ┌── Subscription Worker 1
  HFS Instance B ──┼── Kafka topic ──────────►├── Subscription Worker 2
  HFS Instance C ──┘   (resource-events)       └── Subscription Worker 3
                        partitioned by               (consumer group)
                        resource type + id
```

**Why Kafka is a good fit for FHIR Subscriptions:**

- **Decouples write path from notification path.** Handlers publish an event and return immediately — no HTTP dispatch, no retry loops, no spawned tasks. Write latency is unaffected regardless of how many subscriptions exist or how slow their endpoints are.
- **Ordering guarantees.** Kafka partitions preserve strict ordering within a partition. Partitioning by `(resource_type, resource_id)` ensures that all events for a given resource are processed in order, so subscribers see monotonically increasing `eventNumber`s.
- **Scalable consumer group.** Multiple subscription workers share the load via Kafka's consumer group protocol. Adding workers increases throughput without code changes. Kafka handles rebalancing automatically on scale-up/down.
- **Durable retry without sleeping tasks.** Failed deliveries can be published to a retry topic (or dead-letter topic) with a delay, rather than holding a task alive with `tokio::time::sleep`. The worker is freed immediately to process the next event.
- **Cluster-safe by design.** All HFS instances publish to the same topic. The consumer group ensures each event is processed exactly once — no duplicate notifications, no split counters, no handshake races.
- **Batch/transaction gap closes naturally.** Any code path that writes a resource (including batch and transaction handlers) just needs to publish an event to Kafka. The subscription evaluation happens downstream, so there is no need to wire `emit_subscription_event` into every handler individually.
- **Replay and debugging.** Kafka retains events for a configurable period. Operators can replay events to re-evaluate subscriptions after a bug fix, or inspect the event stream to diagnose why a notification was or wasn't sent.
- **Backpressure is built in.** If workers fall behind, Kafka buffers events durably on disk. Consumer lag is observable via standard Kafka metrics, giving operators clear visibility into subscription processing health.

**Trade-offs:** Kafka adds operational complexity (broker cluster, ZooKeeper/KRaft, topic configuration, monitoring). For single-node or small deployments the in-memory engine remains simpler and sufficient. Kafka is most justified when HFS is deployed as a clustered service handling high write volumes or when notification reliability is critical.

**AWS alternative:** [Amazon MSK](https://aws.amazon.com/msk/) (Managed Streaming for Apache Kafka) provides a fully managed Kafka-compatible service, eliminating broker and ZooKeeper/KRaft operational overhead. For deployments that don't need Kafka's full feature set, [Amazon SQS](https://aws.amazon.com/sqs/) with FIFO queues offers a simpler alternative — FIFO queues provide exactly-once processing and strict ordering within a message group (analogous to a Kafka partition), which maps well to partitioning by `(resource_type, resource_id)`. SQS FIFO requires no cluster management and scales automatically, making it a pragmatic choice for AWS-native deployments where Kafka's replay and retention capabilities are not required.

## Current Limitations

- FHIRPath filter criteria are not evaluated — Phase 2 uses direct JSON field matching only
- Heartbeat delivery is not yet implemented — the `heartbeat_period` field is stored but no background task fires heartbeats
- Batch and transaction bundle entries do not emit subscription events — only direct CRUD handlers (create, update, delete, patch) do
- [`eventTrigger`](https://hl7.org/fhir/subscriptiontopic.html) is not supported — only `resourceTrigger` (create, update, delete) is implemented
- The engine is in-memory only and single-instance — subscriptions and topics are not shared across cluster nodes or reloaded from storage on restart (see [Clustering](#clustering) above)
- WebSocket notifications are best-effort — if no clients are connected when an event fires the notification is silently dropped; there is no replay or queueing for late-connecting clients
- Email and FHIR messaging channels are not yet implemented (planned for subsequent phases)
