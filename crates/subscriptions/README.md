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

The `SubscriptionEngine` orchestrates all five concerns. HFS feeds committed create, update, patch, and delete notices through `SubscriptionWriteObserver`: lifecycle changes apply synchronously, while data notifications enter a bounded, nonblocking dispatcher.

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
| `HFS_SUBSCRIPTION_EVENT_QUEUE_CAPACITY` | `1024` | Resource events waiting for evaluation |
| `HFS_SUBSCRIPTION_MAX_PENDING_NOTIFICATIONS` | `1024` | Global queued + active notifications, including retry sleeps |
| `HFS_SUBSCRIPTION_PER_SUBSCRIPTION_QUEUE_CAPACITY` | `32` | Queued + active notifications for one `(tenant, subscription_id)` |
| `HFS_SUBSCRIPTION_DISPATCH_CONCURRENCY` | `64` | Simultaneous channel attempts, shared by notifications and activation handshakes |
| `HFS_SUBSCRIPTION_HANDSHAKE_CONCURRENCY` | `8` | Running activation flows, shared by live writes and startup rehydration |
| `max_retries` *(config field)* | `10` | Delivery attempts per notification; `0` disables retries |
| `retry_initial_delay` *(config field)* | `1s` | Base delay for delivery backoff |
| `retry_max_delay` *(config field)* | `60s` | Maximum delivery backoff delay |
| `HFS_SUBSCRIPTION_HANDSHAKE_INITIAL_DELAY_MS` | `0` | Delay before the first activation handshake attempt |
| `HFS_SUBSCRIPTION_HANDSHAKE_MAX_ATTEMPTS` | `1` | Max activation handshake attempts before marking error |
| `HFS_SUBSCRIPTION_HANDSHAKE_RETRY_BASE_MS` | `1000` | Initial delay before retrying a failed activation handshake |
| `HFS_SUBSCRIPTION_HANDSHAKE_RETRY_MAX_MS` | `60000` | Maximum delay cap for activation handshake retries |
| `heartbeat_check_interval` *(config field)* | `30s` | Heartbeat check interval |
| `error_threshold` *(config field)* | `3` | Exhausted or permanently failed notifications before `error` |
| `off_threshold` *(config field)* | `10` | Exhausted or permanently failed notifications before `off` |
| `ws_token_lifetime_secs` *(config field)* | `30` | Binding token expiry in seconds (WebSocket only) |

The five dispatch limits accept positive integers up to Tokio's `Semaphore::MAX_PERMITS`; surrounding whitespace is ignored. Missing settings use defaults. Invalid text, zero, negative values, integer overflow, or values above that maximum log `Invalid subscription dispatch limit; using default` and use the corresponding default. Direct `SubscriptionConfig` construction applies the same zero/oversize fallback without requiring a runtime. The engine clamps the per-subscription capacity to the global notification budget and reports effective values through `dispatch_snapshot()`.

Delivery retry, heartbeat, and circuit-breaker fields in this table are programmatic settings; HFS does not parse corresponding retry/heartbeat/threshold environment variables.

### Startup rehydration

The engine's registries are in-memory, so without rehydration a restart would leave every previously-stored subscription inert — the resources still read back over the REST API, but nothing matches or delivers. At startup HFS pages stored `SubscriptionTopic`, R4-backport `Basic`, and `Subscription` resources back into the engine, for every provisioned tenant.

This runs in the background: the server begins accepting requests immediately, and subscriptions come back online shortly after. A summary line (`Subscription engine rehydrated`) reports what was restored.

HFS persists server-driven status transitions by default (`HFS_SUBSCRIPTION_PERSIST_STATUS=true`); a successful handshake updates the stored resource to `active`. Rehydration registers stored `active` subscriptions without another handshake, reactivates `requested` and `error`, and registers `off`/`entered-in-error` dormant so status queries still answer. Set `HFS_SUBSCRIPTION_REHYDRATE_HANDSHAKE=false` to register without outbound handshakes, leaving `requested`/`error` inert until rewritten. Startup handshakes use both the per-rehydration concurrency setting below and the engine-wide activation/attempt limits above.

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_SUBSCRIPTION_REHYDRATE` | `true` | Reload stored subscriptions and topics at startup |
| `HFS_SUBSCRIPTION_REHYDRATE_BATCH_SIZE` | `500` | Resources fetched per storage round-trip |
| `HFS_SUBSCRIPTION_REHYDRATE_HANDSHAKE` | `true` | Re-run the activation handshake for stored `requested` subscriptions |
| `HFS_SUBSCRIPTION_REHYDRATE_HANDSHAKE_CONCURRENCY` | `8` | Max handshakes in flight at once, across all tenants |

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

The engine integrates into `helios-rest` through `AppState` and the post-commit `WriteObservers` fan-out. `SubscriptionWriteObserver::on_write` must stay cheap and nonblocking: it applies Subscription/topic lifecycle notices synchronously and reserves data-ingress capacity before cloning the resource payload. Ordinary resource notifications are best-effort admissions; queue-full shedding never rolls back the already-committed write.

Direct library callers can await `engine.on_resource_event(event)` for completion of admitted deliveries or activation. A shed, cancelled, or unmatched event also completes; inspect `dispatch_snapshot()` for overload counters. Cancelling the caller's future does not retract accepted work, which continues until it finishes, its generation is cancelled, or the engine is dropped.

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

### Bounded delivery and ordering

One ingress consumer evaluates data events and admits notifications without awaiting outbound I/O. Each nonempty `(tenant, subscription_id)` queue has one FIFO worker. A failing receiver therefore holds its own worker and retained notifications; healthy receivers can proceed when global capacity and channel-attempt slots are available. The global notification budget includes queued jobs, active sends, and retry sleeps. At most that many notification workers exist, including workers retiring after cancellation. Retry sleeps release the shared channel-attempt slot.

Retained payload-bearing work is bounded by **ingress capacity + one evaluating event + global notification budget + running activation-flow limit**. The evaluating event can temporarily hold matching subscription snapshots. These are object-count limits, not byte quotas: resource/bundle sizes still determine memory usage. Registries and one coalesced pending activation record per registered subscription are separate from that payload bound. Activation updates replace the pending generation instead of creating a task or retaining a payload for every write.

Admission uses **drop-new** at ingress, global notification, and per-subscription capacity. The synchronous observer never blocks a committed write. A new notification may also be shed temporarily while cancelled workers retire and occupy the bounded worker slots. Fixed warning reasons identify each policy:

- `Subscription resource event shed: ingress queue full (drop-new)`
- `Subscription resource event shed: ingress closed`
- `Subscription notification shed: global queue full (drop-new)`
- `Subscription notification shed: subscription queue full (drop-new)`
- `Subscription notification shed: worker retirement limit (drop-new)`

`dispatch_snapshot()` exposes effective limits, coherent occupancy and cumulative counters (`dropped_ingress_full`, `dropped_ingress_closed`, `dropped_global_full`, `dropped_subscription_full`, `dropped_worker_full`). `resident_notifications` is the authoritative retained-notification gauge. These process-local diagnostics are not exported as new Prometheus series.

Ingress shedding happens before evaluation and consumes no event number. Once a subscription matches, its number advances even if notification admission fails; receivers can therefore observe gaps. Admitted events are sent FIFO within the current subscription generation, and a retry cannot be overtaken by the next event. Shedding does not increment receiver failure streaks or delivery-failure statistics. Circuit-breaker failures remain counted once per exhausted/permanently failed notification, not per HTTP attempt. `error` blocks new matches while already-admitted notifications drain; terminal `off` cancels their remainder.

A successful registration/update invalidates previous queued work and results. Ordinary edits retain counters; requested rearm resets them. Delete/recreate obtains a fresh generation. Cancellation stops pending sends/retries and prevents stale outcomes or status write-back from changing the replacement; it cannot retract a request already sent to a receiver. Status write-back also checks the originating stored version and content, so a newer client write is protected before its observer notice arrives.

Construction outside Tokio is supported. Workers bind lazily to the first dispatch's runtime; keep that runtime alive for the engine lifetime and use one engine per runtime. Observer notices received with no Tokio runtime are not dispatched. Dropping the engine closes its queues and pending completion receipts.

Delivery queues and counters are volatile: this does not provide durable/restart-safe delivery or replay. It does not change downstream WebSocket client buffers or make disconnected WebSocket delivery reliable.

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
