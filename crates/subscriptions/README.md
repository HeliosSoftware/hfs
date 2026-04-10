# helios-subscriptions

FHIR topic-based Subscriptions engine for the Helios FHIR Server, implementing the [FHIR Subscriptions Framework](https://build.fhir.org/subscriptions.html) across R4, R4B, R5, and R6.

R4 and R4B use the [Subscriptions R5 Backport IG](https://build.fhir.org/ig/HL7/fhir-subscription-backport-ig/); R5 and R6 use the native Subscriptions Framework.

## Overview

This crate implements topic-based subscriptions as an asynchronous pipeline that fires after every resource write. It decomposes into five concerns:

1. **Topic Registry** — stores `SubscriptionTopic` definitions and evaluates resource triggers
2. **Subscription Manager** — tracks active `Subscription` resources and their runtime state
3. **Event Evaluator** — matches write events against active subscriptions using topic triggers and filter criteria
4. **Notification Builder** — constructs version-specific notification bundles (R4 Parameters-based backport, R5/R6 native `SubscriptionStatus`)
5. **Channel Dispatcher** — delivers notifications via pluggable channel implementations

The [`SubscriptionEngine`] orchestrates all five concerns and is the main entry point, invoked via `tokio::spawn` after each resource write — mirroring the fire-and-forget pattern used by the audit middleware.

## Features

- **Version-aware parsing**: R4 reads the backport extension set (`backport-topic-canonical`, `backport-payload-content`, `backport-channel-type`, `backport-filter-criteria`); R5/R6 read native `topic`, `channelType`, `filterBy` fields
- **Status state machine**: `Requested → Active → Error → Off` with validated transitions; subscriptions activate only after a successful handshake
- **Exponential backoff retry**: configurable initial delay, max delay, backoff factor, and max attempts before transitioning to `error` or `off`
- **Tenant isolation**: all in-memory maps are keyed by `(tenant_id, subscription_id)` — subscriptions in different tenants never interact
- **TLS enforcement**: `full-resource` payload subscriptions over non-HTTPS endpoints are rejected at dispatch time
- **Pluggable channels**: `ChannelDispatcher` trait allows new channel types (WebSocket, email, FHIR messaging) to be added without touching the engine

## Channel Support

| Channel | Status | Notes |
|---------|--------|-------|
| `rest-hook` | Implemented | HTTP POST with custom headers, TLS enforcement for full-resource payloads |
| `websocket` | Planned (Phase 2) | Binding token flow, per-subscription client registry |
| `email` | Planned (Phase 3) | SMTP via `lettre` |
| `fhir-messaging` | Planned (Phase 4) | Notification wrapped in a FHIR message Bundle |

## Architecture

```
ResourceEvent
     │
     ▼
SubscriptionEngine.on_resource_event()
     │
     ├─ resource_type == "Subscription"   → SubscriptionManager.register() / deregister()
     ├─ resource_type == "SubscriptionTopic" → InMemoryTopicRegistry.add_topic()
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

### R5/R6 (Native)

```json
{
  "resourceType": "Bundle",
  "type": "subscription-notification",
  "entry": [
    {
      "resource": {
        "resourceType": "SubscriptionStatus",
        "status": "active",
        "type": "event-notification",
        "eventsSinceSubscriptionStart": "3",
        "subscription": { "reference": "Subscription/sub-1" },
        "topic": "http://example.org/topic/encounter-start",
        "notificationEvent": [
          {
            "eventNumber": "3",
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

Filters use the R4 backport string format `ResourceType?parameter=value` (parsed by `parse_filter_string`) or the native R5 `filterBy` array. The evaluator supports:

| Filter parameter | Resolved from |
|-----------------|---------------|
| `code` | `CodeableConcept.coding[].code` tokens |
| `category` | `CodeableConcept.coding[].code` tokens |
| `patient` / `subject` | `subject.reference` or `patient.reference` |
| `identifier` | `identifier[].value` |
| *(other)* | Direct JSON field lookup by name |

Comparators supported: `eq` (default), `in`. FHIRPath evaluation is not used in Phase 1 — filters operate on the raw resource JSON.

## Configuration

`SubscriptionConfig` is constructed programmatically or from environment variables when running inside HFS:

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_SUBSCRIPTIONS_ENABLED` | `false` | Enable the subscription engine |
| `HFS_SUBSCRIPTION_MAX_RETRIES` | `10` | Max delivery attempts before marking error |
| `HFS_SUBSCRIPTION_RETRY_INITIAL_DELAY` | `1s` | Initial delay for exponential backoff |
| `HFS_SUBSCRIPTION_RETRY_MAX_DELAY` | `60s` | Maximum delay cap for backoff |
| `HFS_SUBSCRIPTION_HEARTBEAT_INTERVAL` | `30s` | How often to check for due heartbeats |
| `HFS_SUBSCRIPTION_ERROR_THRESHOLD` | `3` | Consecutive failures before `error` status |
| `HFS_SUBSCRIPTION_OFF_THRESHOLD` | `10` | Consecutive failures before `off` status |

## Enabling in HFS

The subscription engine is an optional feature in `helios-rest` and `helios-hfs`:

```bash
# Build with subscriptions support
cargo build --bin hfs --features subscriptions

# Enable at runtime
HFS_SUBSCRIPTIONS_ENABLED=true cargo run --bin hfs --features subscriptions
```

When enabled, the engine auto-initializes with default configuration and begins processing events after the first resource write.

## Integration

The engine integrates into `helios-rest` via the `AppState::with_subscription_engine()` builder. After each successful write, handlers call `emit_subscription_event()` which spawns an async task:

```rust,ignore
// In create handler (simplified)
if let Some(engine) = state.subscription_engine() {
    emit_subscription_event(engine, tenant.context(), &stored, fhir_version, ResourceEventType::Create);
}
```

The spawned task calls `engine.on_resource_event(event).await`, which runs the full evaluation → notification → dispatch pipeline without blocking the HTTP response.

### Registering a Topic

POST a `SubscriptionTopic` resource (R5/R6) or a `Basic` resource with the backport profile (R4) to your HFS instance. The engine picks it up automatically on the next write:

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

The server will immediately send a handshake notification to the endpoint. On a successful 2xx response the subscription transitions to `active`.

### Checking Subscription Status

```bash
GET /Subscription/{id}/$status
```

Returns a `Parameters` resource (R4) or `SubscriptionStatus` resource (R5/R6) with the current runtime status and event count.

## Features

FHIR version support via Cargo feature flags:

| Feature | Default | Description |
|---------|---------|-------------|
| `R4` | Yes | FHIR R4 with Subscriptions R5 Backport IG |
| `R4B` | No | FHIR R4B with Subscriptions R5 Backport IG |
| `R5` | No | FHIR R5 with native Subscriptions Framework |
| `R6` | No | FHIR R6 with native Subscriptions Framework |

```toml
[dependencies]
helios-subscriptions = { version = "0.1", features = ["R4"] }
```

## Current Limitations

- FHIRPath filter criteria are not evaluated — Phase 1 uses direct JSON field matching only
- Heartbeat delivery is not yet implemented — the `heartbeat_period` field is stored but no background task fires heartbeats
- Batch and transaction bundle entries do not emit subscription events — only direct CRUD handlers (create, update, delete, patch) do
- The engine is in-memory only; subscriptions and topics are not reloaded from storage on restart
- Only the `rest-hook` channel is implemented; WebSocket, email, and FHIR messaging are planned for subsequent phases
