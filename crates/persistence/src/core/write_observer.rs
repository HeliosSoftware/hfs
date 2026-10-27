//! Post-commit write events (#1078).
//!
//! Every path that changes stored resources — the REST handlers, batch and
//! transaction bundles, `$bulk-submit` ingestion, tenant conformance seeding,
//! and purges — reports what it committed to one [`WriteObserver`] instead of
//! calling each consumer by hand. The server fans the events out to whoever
//! subscribed (the dashboard's live counters, the subscriptions engine).
//!
//! An event carries two independent facts about a write, because consumers
//! care about different things and the rules that decide each must not bleed
//! into the other:
//!
//! - [`ResourceWrite::live_delta`] — how the number of live resources of the
//!   type changed. Counters read only this.
//! - [`ResourceWrite::notice`] — what resource notifications should say about
//!   the write, decided by the write path exactly as notifications were decided
//!   before this funnel existed. `None` is a deliberate "do not notify" (for
//!   example conditional writes), not a missing value.
//!
//! Observers are called synchronously right after the write committed, on the
//! request or worker thread: implementations must be cheap and must never
//! block (spawn for async work).

use std::fmt;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use helios_fhir::FhirVersion;
use serde_json::Value;

use crate::tenant::TenantId;

/// The kind of change a resource notification describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteKind {
    /// A new resource (including an update that created or restored one).
    Create,
    /// A new version of an existing live resource.
    Update,
    /// A live resource removed.
    Delete,
}

/// What resource notifications should report for one write.
#[derive(Debug, Clone)]
pub struct WriteNotice {
    /// Create, update or delete, as the notification should name it.
    pub kind: WriteKind,
    /// Logical id of the resource.
    pub resource_id: String,
    /// Version written; empty for deletes.
    pub version_id: String,
    /// The written content (with meta) for creates and updates; `None` for
    /// deletes.
    pub resource: Option<Value>,
    /// The content before the write, when the write path had it.
    pub previous: Option<Value>,
}

/// One committed write of one resource.
#[derive(Debug, Clone)]
pub struct ResourceWrite {
    /// Tenant the write belongs to.
    pub tenant: TenantId,
    /// FHIR version the resource was written as.
    pub fhir_version: FhirVersion,
    /// Resource type written.
    pub resource_type: String,
    /// Net change in live resources of `resource_type`: `+1` when a resource
    /// was created (including update-as-create and restore), `-1` when a live
    /// resource was deleted, `0` otherwise.
    pub live_delta: i64,
    /// What resource notifications should say; `None` means do not notify.
    pub notice: Option<WriteNotice>,
    /// When the write committed.
    pub at: DateTime<Utc>,
}

/// Where an aggregate [`WriteEvent::Counts`] comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOrigin {
    /// A committed `$bulk-submit` ingestion batch, or a deletes pass.
    BulkSubmit,
    /// Conformance resources seeded into a tenant.
    ConformanceSeed,
}

/// What a purge erased.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErasedScope {
    /// One resource and its history (`$purge` on an instance).
    Instance {
        /// Resource type purged.
        resource_type: String,
        /// Logical id purged.
        id: String,
    },
    /// Every resource of one type (`$purge` on a type).
    Type(String),
    /// All of the tenant's data.
    Tenant,
}

/// A committed change to stored resources.
#[derive(Debug, Clone)]
pub enum WriteEvent {
    /// One resource written.
    Resource(ResourceWrite),
    /// Many resources of one type written at once, reported as counts only
    /// (no per-resource notification is sent for these).
    Counts {
        /// Tenant the writes belong to.
        tenant: TenantId,
        /// Resource type written.
        resource_type: String,
        /// Resources created.
        created: u64,
        /// Existing resources updated.
        updated: u64,
        /// Live resources deleted.
        deleted: u64,
        /// Which path produced the writes.
        origin: WriteOrigin,
        /// When the writes committed.
        at: DateTime<Utc>,
    },
    /// Data erased by a purge; consumers holding figures for it must treat
    /// them as stale.
    Erased {
        /// Tenant whose data was erased.
        tenant: TenantId,
        /// What was erased.
        scope: ErasedScope,
    },
    /// The tenant was deregistered (its data may still exist). Consumers
    /// holding state for it can drop that state; a later use rebuilds it.
    TenantRemoved {
        /// Tenant that was deregistered.
        tenant: TenantId,
    },
}

impl WriteEvent {
    /// The tenant the event belongs to.
    pub fn tenant(&self) -> &TenantId {
        match self {
            WriteEvent::Resource(write) => &write.tenant,
            WriteEvent::Counts { tenant, .. }
            | WriteEvent::Erased { tenant, .. }
            | WriteEvent::TenantRemoved { tenant } => tenant,
        }
    }
}

/// Receives committed write events. See the [module documentation](self).
pub trait WriteObserver: Send + Sync {
    /// Called right after the write committed. Must be cheap and must not
    /// block.
    fn on_write(&self, event: &WriteEvent);

    /// The [`WriteObservers`] fan-out this observer is, if it is one, so a
    /// component handed the server's observer as `dyn WriteObserver` can
    /// subscribe to it too (the web UI's tenant inventory, #1850). Default
    /// `None`.
    ///
    /// The web UI subscribes through this hook when it is mounted, instead of
    /// the server wiring each mount by hand.
    fn fan_out(&self) -> Option<&WriteObservers> {
        None
    }

    /// The observer will never act on an event again (it forwards to
    /// something that is gone), so a fan-out may drop it. Default `false`.
    ///
    /// [`WriteObservers`] has no unsubscribe: an observer whose consumer can
    /// be torn down (a remounted UI's tenant inventory, held weakly) reports
    /// itself retired, and the fan-out drops it on the next subscription.
    fn retired(&self) -> bool {
        false
    }
}

/// Fans every event out to the observers that subscribed, in subscription
/// order. This is the single observer the write paths hold.
#[derive(Default)]
pub struct WriteObservers {
    observers: RwLock<Vec<Arc<dyn WriteObserver>>>,
}

impl WriteObservers {
    /// An empty fan-out.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an observer; it receives every event reported from now on.
    /// Observers that report themselves [retired](WriteObserver::retired) are
    /// dropped first, so repeated mounts do not accumulate dead subscribers.
    pub fn subscribe(&self, observer: Arc<dyn WriteObserver>) {
        let mut observers = self
            .observers
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        observers.retain(|existing| !existing.retired());
        observers.push(observer);
    }

    /// How many observers subscribed.
    pub fn len(&self) -> usize {
        self.observers
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    /// Whether nobody subscribed.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl WriteObserver for WriteObservers {
    fn fan_out(&self) -> Option<&WriteObservers> {
        Some(self)
    }

    fn on_write(&self, event: &WriteEvent) {
        // Clone the list out so an observer that subscribes another one from
        // inside `on_write` cannot deadlock on the lock.
        let observers: Vec<Arc<dyn WriteObserver>> = self
            .observers
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        for observer in observers {
            observer.on_write(event);
        }
    }
}

impl fmt::Debug for WriteObservers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WriteObservers")
            .field("observers", &self.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct Recording {
        seen: Mutex<Vec<String>>,
    }

    impl WriteObserver for Recording {
        fn on_write(&self, event: &WriteEvent) {
            let label = match event {
                WriteEvent::Resource(write) => format!("resource:{}", write.resource_type),
                WriteEvent::Counts { resource_type, .. } => format!("counts:{resource_type}"),
                WriteEvent::Erased { .. } => "erased".to_string(),
                WriteEvent::TenantRemoved { .. } => "removed".to_string(),
            };
            self.seen.lock().unwrap().push(label);
        }
    }

    fn tenant() -> TenantId {
        TenantId::new("t1".to_string())
    }

    /// #1850: a holder of the fan-out as `dyn WriteObserver` can reach it to
    /// subscribe; any other observer is not a fan-out.
    #[test]
    fn a_fan_out_held_as_a_plain_observer_can_be_subscribed_to() {
        let observers = Arc::new(WriteObservers::new());
        let held: Arc<dyn WriteObserver> = observers.clone();
        let late = Arc::new(Recording::default());
        held.fan_out()
            .expect("the fan-out exposes itself")
            .subscribe(late.clone());
        assert_eq!(observers.len(), 1);
        held.on_write(&WriteEvent::TenantRemoved { tenant: tenant() });
        assert_eq!(*late.seen.lock().unwrap(), vec!["removed".to_string()]);
        assert!(Recording::default().fan_out().is_none());
    }

    /// An observer that reports itself retired.
    struct Retired;

    impl WriteObserver for Retired {
        fn on_write(&self, _event: &WriteEvent) {
            panic!("a retired observer is never called after a subscription");
        }

        fn retired(&self) -> bool {
            true
        }
    }

    /// #1850: a new subscription drops retired observers, so remounting a
    /// consumer does not accumulate dead subscribers.
    #[test]
    fn subscribing_drops_retired_observers() {
        let observers = WriteObservers::new();
        observers.subscribe(Arc::new(Retired));
        observers.subscribe(Arc::new(Retired));
        assert_eq!(observers.len(), 1, "only the latest retired one is left");
        let live = Arc::new(Recording::default());
        observers.subscribe(live.clone());
        assert_eq!(observers.len(), 1);
        assert!(!live.retired());
        observers.on_write(&WriteEvent::TenantRemoved { tenant: tenant() });
        assert_eq!(*live.seen.lock().unwrap(), vec!["removed".to_string()]);
    }

    #[test]
    fn fans_out_every_event_to_every_observer_in_order() {
        let observers = WriteObservers::new();
        assert!(observers.is_empty());
        let first = Arc::new(Recording::default());
        let second = Arc::new(Recording::default());
        observers.subscribe(first.clone());
        observers.subscribe(second.clone());
        assert_eq!(observers.len(), 2);

        observers.on_write(&WriteEvent::Counts {
            tenant: tenant(),
            resource_type: "Patient".to_string(),
            created: 3,
            updated: 1,
            deleted: 0,
            origin: WriteOrigin::BulkSubmit,
            at: Utc::now(),
        });
        observers.on_write(&WriteEvent::Erased {
            tenant: tenant(),
            scope: ErasedScope::Tenant,
        });

        for recording in [&first, &second] {
            assert_eq!(
                *recording.seen.lock().unwrap(),
                vec!["counts:Patient".to_string(), "erased".to_string()]
            );
        }
    }

    #[test]
    fn an_event_names_its_tenant() {
        let event = WriteEvent::Erased {
            tenant: tenant(),
            scope: ErasedScope::Type("Observation".to_string()),
        };
        assert_eq!(event.tenant().as_str(), "t1");
    }
}
