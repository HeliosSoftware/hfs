//! One optional soft deadline, checked between MongoDB commands and matching batches.

use std::{future::Future, time::Duration};

use mongodb::action::{Aggregate, CountDocuments, Distinct, Find, FindOne};
use tokio::time::Instant;

use crate::error::{BackendError, StorageError, StorageResult};

tokio::task_local! {
    static DEADLINE: Instant;
}

fn timeout_error() -> StorageError {
    BackendError::Timeout {
        backend_name: "mongodb".to_owned(),
        message: "MongoDB search deadline expired".to_owned(),
    }
    .into()
}

pub(super) fn is_active() -> bool {
    DEADLINE.try_with(|_| ()).is_ok()
}

pub(super) fn check() -> StorageResult<()> {
    if DEADLINE
        .try_with(|expires| Instant::now() >= *expires)
        .unwrap_or(false)
    {
        Err(timeout_error())
    } else {
        Ok(())
    }
}

fn remaining() -> StorageResult<Option<Duration>> {
    DEADLINE
        .try_with(|expires| {
            let remaining = expires.saturating_duration_since(Instant::now());
            // MongoDB encodes whole milliseconds; zero would disable maxTimeMS.
            if remaining.as_millis() == 0 {
                return Err(timeout_error());
            }
            Ok(Some(Duration::from_millis(remaining.as_millis() as u64)))
        })
        .unwrap_or(Ok(None))
}

/// Retain which budget stopped a probe, including server-side rounding.
pub(super) struct ProbeBudget {
    duration: Duration,
    limited_by_search: bool,
}

impl ProbeBudget {
    pub(super) fn new(probe: Duration) -> StorageResult<Self> {
        let left = remaining()?;
        Ok(Self {
            duration: left.map_or(probe, |left| left.min(probe)),
            limited_by_search: left.is_some_and(|left| left <= probe),
        })
    }

    pub(super) fn duration(&self) -> Duration {
        self.duration
    }

    pub(super) fn check_timeout(&self, timed_out: bool) -> StorageResult<()> {
        check()?;
        if timed_out && self.limited_by_search {
            return Err(timeout_error());
        }
        Ok(())
    }
}

/// Admission is cancellation-safe; database futures must run to completion.
pub(super) async fn wait<T>(future: impl Future<Output = T>) -> StorageResult<T> {
    match DEADLINE.try_with(|expires| *expires) {
        Ok(expires) => tokio::time::timeout_at(expires, future)
            .await
            .map_err(|_| timeout_error()),
        Err(_) => Ok(future.await),
    }
}

/// A command or batch may overrun, and work that finishes late is returned:
/// the deadline stops new work from starting, never discards finished work,
/// and never drops an in-flight driver future.
pub(super) async fn run<T>(
    timeout_ms: u64,
    work: impl Future<Output = StorageResult<T>>,
) -> StorageResult<T> {
    let checked = async {
        check()?;
        work.await
    };
    if is_active() {
        checked.await
    } else {
        DEADLINE
            .scope(Instant::now() + Duration::from_millis(timeout_ms), checked)
            .await
    }
}

/// Add maxTimeMS only inside a configured search, leaving other callers alone.
pub(super) trait SearchCommand: Sized {
    fn search_deadline(self) -> StorageResult<Self>;

    fn search_probe_deadline(self, probe: Duration) -> StorageResult<(Self, ProbeBudget)>;
}

macro_rules! search_command {
    ($action:ty, $($generic:tt)*) => {
        impl<$($generic)*> SearchCommand for $action {
            fn search_probe_deadline(self, probe: Duration) -> StorageResult<(Self, ProbeBudget)> {
                let budget = ProbeBudget::new(probe)?;
                Ok((self.max_time(budget.duration()), budget))
            }

            fn search_deadline(self) -> StorageResult<Self> {
                Ok(match remaining()? {
                    Some(budget) => self.max_time(budget),
                    None => self,
                })
            }
        }
    };
}

search_command!(Find<'a, T, S>, 'a, T: Send + Sync, S);
search_command!(FindOne<'a, T>, 'a, T: Send + Sync);
search_command!(Aggregate<'a, S, T>, 'a, S, T);
search_command!(CountDocuments<'a>, 'a);
search_command!(Distinct<'a>, 'a);

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::{Semaphore, oneshot};

    fn assert_timeout<T: std::fmt::Debug>(result: StorageResult<T>) {
        assert!(
            matches!(
                result,
                Err(StorageError::Backend(BackendError::Timeout { .. }))
            ),
            "{result:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn search_deadline_is_absent_outside_a_search() {
        assert!(!is_active());
        assert_eq!(remaining().unwrap(), None);
        let probe = ProbeBudget::new(Duration::from_millis(10)).unwrap();
        assert_eq!(probe.duration(), Duration::from_millis(10));
        probe.check_timeout(true).unwrap();
        assert_eq!(wait(async { 7 }).await.unwrap(), 7);
    }

    #[tokio::test(start_paused = true)]
    async fn search_deadline_decreases_across_steps_and_never_emits_zero() {
        DEADLINE
            .scope(Instant::now() + Duration::from_millis(20), async {
                assert_eq!(remaining().unwrap(), Some(Duration::from_millis(20)));
                tokio::time::advance(Duration::from_millis(12)).await;
                assert_eq!(remaining().unwrap(), Some(Duration::from_millis(8)));
                tokio::time::advance(Duration::from_micros(7_500)).await;
                assert_timeout(remaining());
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn search_deadline_probe_expiry_does_not_become_a_fallback() {
        DEADLINE
            .scope(Instant::now() + Duration::from_millis(20), async {
                let optional = ProbeBudget::new(Duration::from_millis(10)).unwrap();
                optional.check_timeout(true).unwrap();
                tokio::time::advance(Duration::from_millis(15)).await;
                let capped = ProbeBudget::new(Duration::from_millis(10)).unwrap();
                assert_eq!(capped.duration(), Duration::from_millis(5));
                assert_timeout(capped.check_timeout(true));
                capped.check_timeout(false).unwrap();
                tokio::time::advance(Duration::from_millis(5)).await;
                assert_timeout(optional.check_timeout(false));
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn search_deadline_is_soft_and_keeps_in_flight_work_until_completion() {
        let gate = Arc::new(Semaphore::new(1));
        let worker_gate = gate.clone();
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let task = tokio::spawn(run(20, async move {
            let _permit = worker_gate.acquire().await.unwrap();
            started_tx.send(()).unwrap();
            release_rx.await.unwrap();
            Ok(42)
        }));
        started_rx.await.unwrap();
        tokio::time::advance(Duration::from_millis(20)).await;
        tokio::task::yield_now().await;
        assert!(
            !task.is_finished(),
            "soft expiry must not interrupt in-flight work"
        );
        assert_eq!(gate.available_permits(), 0);
        release_tx
            .send(())
            .expect("in-flight work was not cancelled");
        assert_eq!(task.await.unwrap().unwrap(), 42, "late work is returned");
        assert_eq!(gate.available_permits(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn search_deadline_bounds_admission_and_inherits_the_original_scope() {
        let gate = Semaphore::new(0);
        assert_timeout(
            run(20, async {
                let _permit = wait(gate.acquire()).await?.unwrap();
                Ok(())
            })
            .await,
        );
        run(20, async {
            tokio::time::advance(Duration::from_millis(12)).await;
            run(5000, async {
                assert_eq!(remaining().unwrap(), Some(Duration::from_millis(8)));
                Ok(())
            })
            .await
        })
        .await
        .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn search_deadline_returns_finished_work_and_refuses_new_work_after_expiry() {
        let late = run(20, async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            Ok(159)
        })
        .await;
        assert_eq!(late.unwrap(), 159, "work that finished late is returned");
        run(20, async {
            tokio::time::advance(Duration::from_millis(25)).await;
            assert_timeout(run(5000, async { Ok(1) }).await);
            assert_timeout(check());
            Ok(())
        })
        .await
        .unwrap();
    }
}
