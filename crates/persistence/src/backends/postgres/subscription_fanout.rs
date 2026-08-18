//! [`EventFanout`] implementation for PostgreSQL, over `LISTEN`/`NOTIFY`.
//!
//! Publishing rides the shared pool (`SELECT pg_notify(...)`); listening
//! needs a **dedicated non-pooled** connection because `LISTEN` binds to the
//! session and notifications arrive as async messages on that session's
//! wire — a pooled connection would be reclaimed out from under it. The
//! dedicated connection is built from the backend's retained
//! [`PostgresConfig`](super::PostgresConfig) and driven by a background
//! task with a capped-backoff reconnect loop; after every *re*connect the
//! task synthesizes a local [`FanoutKind::Resync`] so consumers know
//! envelopes may have been missed and can re-hydrate.
//!
//! NOTIFY is a best-effort wake/refresh signal by contract (see
//! [`crate::core::event_fanout`]); nothing here persists or replays
//! envelopes. Note for operators: the dedicated connection speaks the
//! session-level protocol, so a transaction-pooling proxy (pgbouncer in
//! transaction mode) between HFS and Postgres breaks LISTEN.

use async_trait::async_trait;
use futures::StreamExt;
use tokio::sync::{broadcast, mpsc, watch};
use tokio_postgres::{AsyncMessage, NoTls};

use crate::core::event_fanout::{EventFanout, FanoutEnvelope, FanoutKind};
use crate::error::{BackendError, StorageError, StorageResult};

use super::PostgresConfig;

/// NOTIFY channel carrying [`FanoutEnvelope`] JSON.
const EVENTS_CHANNEL: &str = "hfs_subscription_events";
/// NOTIFY channel carrying empty outbox wake hints.
const WAKE_CHANNEL: &str = "hfs_subscription_outbox_wake";

/// Reconnect backoff bounds for the listener loop.
const RECONNECT_BACKOFF_INITIAL_SECS: u64 = 1;
const RECONNECT_BACKOFF_MAX_SECS: u64 = 30;

/// [`EventFanout`] over Postgres `LISTEN`/`NOTIFY`.
///
/// Obtained via `ResourceStorage::subscription_fanout()` on
/// [`super::PostgresBackend`], which memoizes one instance per backend:
/// construction spawns the listener task (so the first accessor call must
/// happen inside a Tokio runtime), and dropping the fan-out stops it.
pub struct PgNotifyFanout {
    pool: deadpool_postgres::Pool,
    envelopes: broadcast::Sender<FanoutEnvelope>,
    wakes: broadcast::Sender<()>,
    /// `true` while the listener's `LISTEN` session is established.
    connected: watch::Receiver<bool>,
    /// Dropped with `self`; the listener task observes the closure and
    /// exits, releasing its dedicated connection.
    _shutdown: watch::Sender<()>,
}

impl PgNotifyFanout {
    /// Creates the fan-out and spawns its listener task. Must be called
    /// from within a Tokio runtime.
    pub(crate) fn new(pool: deadpool_postgres::Pool, config: PostgresConfig) -> Self {
        let envelopes = broadcast::channel(1024).0;
        let wakes = broadcast::channel(1024).0;
        let (shutdown_tx, shutdown_rx) = watch::channel(());
        let (connected_tx, connected_rx) = watch::channel(false);

        tokio::spawn(listener_loop(
            config,
            envelopes.clone(),
            wakes.clone(),
            connected_tx,
            shutdown_rx,
        ));

        Self {
            pool,
            envelopes,
            wakes,
            connected: connected_rx,
            _shutdown: shutdown_tx,
        }
    }

    async fn notify(&self, channel: &str, payload: &str) -> StorageResult<()> {
        let client = self.pool.get().await.map_err(|e| {
            StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: "postgres".to_string(),
                message: e.to_string(),
            })
        })?;
        client
            .execute("SELECT pg_notify($1, $2)", &[&channel, &payload])
            .await
            .map_err(|e| {
                StorageError::Backend(BackendError::Internal {
                    backend_name: "postgres".to_string(),
                    message: format!("pg_notify failed: {e}"),
                    source: None,
                })
            })?;
        Ok(())
    }
}

#[async_trait]
impl EventFanout for PgNotifyFanout {
    async fn ready(&self) {
        let mut connected = self.connected.clone();
        // wait_for resolves immediately when already true; if the listener
        // task is gone (backend dropped mid-await), just return.
        let _ = connected.wait_for(|connected| *connected).await;
    }

    async fn publish(&self, envelope: &FanoutEnvelope) -> StorageResult<()> {
        let payload = serde_json::to_string(envelope).map_err(|e| {
            StorageError::Backend(BackendError::Internal {
                backend_name: "postgres".to_string(),
                message: format!("envelope serialization failed: {e}"),
                source: None,
            })
        })?;
        self.notify(EVENTS_CHANNEL, &payload).await
    }

    fn subscribe(&self) -> broadcast::Receiver<FanoutEnvelope> {
        self.envelopes.subscribe()
    }

    async fn publish_outbox_wake(&self) -> StorageResult<()> {
        self.notify(WAKE_CHANNEL, "").await
    }

    fn subscribe_outbox_wake(&self) -> broadcast::Receiver<()> {
        self.wakes.subscribe()
    }
}

/// Builds the dedicated-connection config from the backend's retained
/// settings. TLS mirrors the pool: `NoTls` (see `create_pool`).
fn listen_config(config: &PostgresConfig) -> tokio_postgres::Config {
    let mut pg = tokio_postgres::Config::new();
    pg.host(&config.host)
        .port(config.port)
        .dbname(&config.dbname)
        .user(&config.user)
        .application_name("hfs-subscription-fanout");
    if let Some(password) = &config.password {
        pg.password(password);
    }
    pg
}

/// The listener task: connect, `LISTEN`, drain notifications into the
/// broadcast channels; on connection loss, back off (capped), reconnect,
/// and synthesize a local [`FanoutKind::Resync`] so consumers re-hydrate.
/// Exits when the owning [`PgNotifyFanout`] is dropped.
async fn listener_loop(
    config: PostgresConfig,
    envelopes: broadcast::Sender<FanoutEnvelope>,
    wakes: broadcast::Sender<()>,
    connected: watch::Sender<bool>,
    mut shutdown: watch::Receiver<()>,
) {
    let pg_config = listen_config(&config);
    let mut announce_resync = false;
    let mut backoff_secs = RECONNECT_BACKOFF_INITIAL_SECS;

    loop {
        let outcome = listen_once(
            &pg_config,
            &envelopes,
            &wakes,
            &connected,
            announce_resync,
            &mut shutdown,
        )
        .await;
        let _ = connected.send(false);
        match outcome {
            ListenOutcome::Shutdown => return,
            ListenOutcome::ConnectionEnded => {
                // We had an established session: reconnect promptly and
                // tell consumers envelopes may have been missed.
                announce_resync = true;
                backoff_secs = RECONNECT_BACKOFF_INITIAL_SECS;
                tracing::warn!("Subscription fan-out LISTEN connection ended; reconnecting");
            }
            ListenOutcome::ConnectFailed(error) => {
                tracing::warn!(
                    error = %error,
                    backoff_secs,
                    "Subscription fan-out LISTEN connection failed; backing off"
                );
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)) => {}
                    _ = shutdown.changed() => return,
                }
                backoff_secs = (backoff_secs * 2).min(RECONNECT_BACKOFF_MAX_SECS);
            }
        }
    }
}

enum ListenOutcome {
    /// The owning fan-out was dropped.
    Shutdown,
    /// A session was established and later lost.
    ConnectionEnded,
    /// The session could not be established.
    ConnectFailed(String),
}

async fn listen_once(
    pg_config: &tokio_postgres::Config,
    envelopes: &broadcast::Sender<FanoutEnvelope>,
    wakes: &broadcast::Sender<()>,
    connected: &watch::Sender<bool>,
    announce_resync: bool,
    shutdown: &mut watch::Receiver<()>,
) -> ListenOutcome {
    let (client, mut connection) = match pg_config.connect(NoTls).await {
        Ok(pair) => pair,
        Err(e) => return ListenOutcome::ConnectFailed(e.to_string()),
    };

    // The connection must be polled for the client to make progress AND it
    // is the source of async notification messages — drive it on a helper
    // task that forwards every message here.
    let (message_tx, mut message_rx) = mpsc::unbounded_channel();
    let driver = tokio::spawn(async move {
        let mut stream = futures::stream::poll_fn(move |cx| connection.poll_message(cx));
        while let Some(message) = stream.next().await {
            match message {
                Ok(message) => {
                    if message_tx.send(message).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Subscription fan-out connection error");
                    break;
                }
            }
        }
    });

    let listen_sql = format!("LISTEN {EVENTS_CHANNEL}; LISTEN {WAKE_CHANNEL};");
    if let Err(e) = client.batch_execute(&listen_sql).await {
        driver.abort();
        return ListenOutcome::ConnectFailed(format!("LISTEN failed: {e}"));
    }

    let _ = connected.send(true);
    if announce_resync {
        let _ = envelopes.send(FanoutEnvelope::new(FanoutKind::Resync));
        tracing::info!("Subscription fan-out reconnected; local resync announced");
    }

    // `client` must stay alive while we drain: dropping it closes the
    // session.
    let outcome = loop {
        tokio::select! {
            message = message_rx.recv() => match message {
                Some(AsyncMessage::Notification(notification)) => {
                    match notification.channel() {
                        EVENTS_CHANNEL => {
                            match serde_json::from_str::<FanoutEnvelope>(notification.payload()) {
                                Ok(envelope) => {
                                    let _ = envelopes.send(envelope);
                                }
                                Err(e) => tracing::warn!(
                                    error = %e,
                                    "Dropping unparseable fan-out envelope"
                                ),
                            }
                        }
                        WAKE_CHANNEL => {
                            let _ = wakes.send(());
                        }
                        other => tracing::debug!(channel = other, "Ignoring unknown NOTIFY"),
                    }
                }
                // Notices and other async messages are uninteresting.
                Some(_) => {}
                None => break ListenOutcome::ConnectionEnded,
            },
            _ = shutdown.changed() => break ListenOutcome::Shutdown,
        }
    };

    drop(client);
    driver.abort();
    outcome
}
