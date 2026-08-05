---
name: work-with-subscriptions
description: Work on the HFS FHIR Subscriptions engine. Use for topic-based Subscriptions, channels (rest-hook/websocket/email/messaging), SubscriptionTopic matching, notification delivery, retry, and subscription configuration.
---

# Subscriptions

Use this when working in `helios-subscriptions`, the FHIR topic-based Subscriptions engine. It is consumed by `helios-rest` and `helios-hfs`. The engine evaluates resource events against `SubscriptionTopic` criteria and delivers notifications over channels.

## Behavior

- Enable with `HFS_SUBSCRIPTIONS_ENABLED=true`.
- Channels: rest-hook (HTTP callback), websocket, email, and messaging.
- The messaging channel is separately gated by `HFS_SUBSCRIPTION_MESSAGING_ENABLED`.
- Endpoint safety: rest-hook delivery to private/loopback endpoints is blocked unless `HFS_SUBSCRIPTION_ALLOW_PRIVATE_ENDPOINTS=true`.
- Notification bundles follow the topic-based subscription spec; delivery failures retry with backoff (`engine/retry.rs`).
- WebSocket subscriptions use binding tokens (`ws_token`) managed per connection (`ws_manager`).

## Channels (`crates/subscriptions/src/channels/`)

`rest_hook`, `websocket` (+ `ws_manager`, `ws_token`), `email`, `messaging`.

## Environment

| Variable | Description |
|---|---|
| `HFS_SUBSCRIPTIONS_ENABLED` | Master switch for the subscriptions engine |
| `HFS_SUBSCRIPTION_MESSAGING_ENABLED` | Enable the messaging channel |
| `HFS_SUBSCRIPTION_ALLOW_PRIVATE_ENDPOINTS` | Allow rest-hook delivery to private/loopback URLs (opt-in) |
| `HFS_BASE_URL` | Base URL used in notification links |
| `HFS_SUBSCRIPTION_PERSIST_STATUS` | Write server-driven status transitions back into the stored `Subscription` (default `true`) |
| `HFS_SUBSCRIPTION_STATUS_WRITE_TIMEOUT_MS` | Per-write timeout for that write-back (default `5000`) |
| `HFS_SUBSCRIPTION_REHYDRATE` | Replay stored topics/subscriptions into the engine at startup (default `true`) |
| `HFS_SUBSCRIPTION_REHYDRATE_HANDSHAKE` | Re-run activation for stored `requested`/`error` subscriptions (default `true`) |
| `HFS_SUBSCRIPTION_REHYDRATE_HANDSHAKE_CONCURRENCY` | Cap on concurrent startup handshakes (default `8`) |
| `HFS_SUBSCRIPTION_REHYDRATE_BATCH_SIZE` | Resources fetched per storage round-trip during rehydration (default `500`) |

Exact defaults are read in `crates/rest/src/lib.rs` and `crates/subscriptions/src/config.rs`.

## Status persistence (issue #357)

Server-driven status transitions (`requested` → `active` after a successful
handshake, `→ error`, `→ off` when the delivery circuit breaker trips) are
written back into the stored `Subscription` resource through the ordinary
`ResourceStorage::update`, so the behaviour is identical on SQLite, PostgreSQL,
MongoDB, S3, and composite deployments with no schema change.

Things to know when working here:

- The write happens only when `SubscriptionManager::update_status` accepts the
  transition, so it is bounded to at most three writes per subscription
  lifetime. `SubscriptionEngine::transition_status` is the single choke point —
  do not call `manager.update_status` directly from the engine.
- It **bumps `meta.versionId`** and appends to `_history`, so a client holding
  an ETag from its own POST can see `412` on a later `If-Match` write. That is
  correct optimistic-locking behaviour, not a bug.
- It fails **open**: a storage error leaves the in-memory transition in place
  and logs at `error`. Grep for `Failed to persist subscription status
  transition` to spot a server that has silently reverted to losing its
  decisions on restart.
- `off` and `entered-in-error` are terminal and are rehydrated **dormant**
  (registered so `$status` still answers, never matched, never handshaken).
  `error` is re-activated on restart, because nothing else ever performs
  `error` → `active`.
- `$status` / `$events` fall back to the stored resource when the engine does
  not hold the subscription, instead of `404`.

## Key API

`SubscriptionEngine`, `SubscriptionConfig` / `MessagingSettings`, `ResourceEvent` / `ResourceEventType`, `WebSocketManager`, `WsBindingTokenManager`, `MessagingChannel`, `SubscriptionError`.

## Code map / tests

`engine/` (+ retry), `evaluator/` (filter_match), `manager/` (filters, status), `notification/` (builder, messaging), `topics/`, `channels/`, `event.rs`, `config.rs`. CI: `.github/workflows/inferno-subscription.yml`, `subscriptions-channels.yml`, `subscriptions-smoke.yml`. Unit tests inline in `src/`; integration tests under `crates/subscriptions/tests/` where present.
