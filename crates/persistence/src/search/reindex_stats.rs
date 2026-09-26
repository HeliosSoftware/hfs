//! Always-on wall-clock accounting of one `run_reindex` call (#1403). Local to
//! the run, so concurrent jobs never mix (unlike `crate::perf`'s
//! process-global counters). Logged by `reindex.rs`; the field contract
//! (`reindex job started` ... `reindex page`) is documented on its log
//! helpers and in the design for #1403.

use std::time::{Duration, Instant};

use super::reindex::ReindexPageStats;

pub(super) const OUTCOME_COMPLETED: &str = "completed";
pub(super) const OUTCOME_CANCELLED: &str = "cancelled";
pub(super) const OUTCOME_FAILED: &str = "failed";
/// Logged as `resource_type` when no type is open: no logged value is ever empty.
pub(super) const NO_TYPE: &str = "-";

/// Whole milliseconds, truncated.
pub(super) fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// `count / elapsed` per second, rounded to one decimal; 0.0 when `elapsed` is zero.
pub(super) fn per_second(count: u64, elapsed: Duration) -> f64 {
    if elapsed.is_zero() {
        return 0.0;
    }
    ((count as f64 / elapsed.as_secs_f64()) * 10.0).round() / 10.0
}

/// What one page cost the driver and its writers, before it is folded into a
/// type's or the job's running [`Counters`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct PageRecord {
    pub(super) resources: u64,
    pub(super) entries: u64,
    pub(super) failed: u64,
    pub(super) fetch: Duration,
    /// Time the driver actually waited for this page: equal to `fetch`
    /// unless the source prefetched it while the previous page was being
    /// written, in which case it is the (possibly zero) time still spent
    /// waiting once the write finished.
    pub(super) fetch_wait: Duration,
    pub(super) write: Duration,
    pub(super) writer: ReindexPageStats,
}

/// Running totals over some scope (a type, or the whole job).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Counters {
    pub(super) resources: u64,
    pub(super) entries: u64,
    pub(super) failed: u64,
    pub(super) pages: u64,
    pub(super) fetch: Duration,
    pub(super) fetch_wait: Duration,
    pub(super) write: Duration,
    pub(super) yielded: Duration,
    pub(super) writer: ReindexPageStats,
}

impl Counters {
    fn add_page(&mut self, page: &PageRecord) {
        self.resources += page.resources;
        self.entries += page.entries;
        self.failed += page.failed;
        self.pages += 1;
        self.fetch += page.fetch;
        self.fetch_wait += page.fetch_wait;
        self.write += page.write;
        self.writer.accumulate(&page.writer);
    }

    fn add_yield(&mut self, d: Duration) {
        self.yielded += d;
    }
}

/// [`Counters`]' phase fields, in whole milliseconds, with the two derived
/// fields (`writer_other_ms`, `other_ms`) computed on `Duration`s before
/// truncation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct PhaseMillis {
    pub(super) fetch_ms: u64,
    pub(super) fetch_wait_ms: u64,
    pub(super) write_ms: u64,
    pub(super) extract_ms: u64,
    pub(super) delete_ms: u64,
    pub(super) insert_ms: u64,
    pub(super) db_wait_ms: u64,
    pub(super) writer_other_ms: u64,
    pub(super) yield_ms: u64,
    pub(super) other_ms: u64,
}

impl PhaseMillis {
    pub(super) fn of(c: &Counters, elapsed: Duration) -> Self {
        let db_wait = c.writer.db_wait_or_busy();
        let writer_busy = c.writer.extract + db_wait;
        let writer_other = c.write.saturating_sub(writer_busy);
        let busy = c.fetch_wait + c.write + c.yielded;
        let other = elapsed.saturating_sub(busy);
        Self {
            fetch_ms: millis(c.fetch),
            fetch_wait_ms: millis(c.fetch_wait),
            write_ms: millis(c.write),
            extract_ms: millis(c.writer.extract),
            delete_ms: millis(c.writer.delete),
            insert_ms: millis(c.writer.insert),
            db_wait_ms: millis(db_wait),
            writer_other_ms: millis(writer_other),
            yield_ms: millis(c.yielded),
            other_ms: millis(other),
        }
    }
}

pub(super) struct TypeStarted {
    pub(super) resource_type: String,
    pub(super) type_index: usize,
    pub(super) types: usize,
    pub(super) type_total: u64,
    /// Job clock at the type's start.
    pub(super) elapsed: Duration,
}

pub(super) struct RecordedPage {
    /// 1-based page number within the open type; 0 when no type is open.
    pub(super) page: u64,
    /// Time since the open type started; zero when no type is open.
    pub(super) type_elapsed: Duration,
    pub(super) progress: Option<ProgressSnapshot>,
}

pub(super) struct ProgressSnapshot {
    /// `NO_TYPE` when no type is open (type fields are then zero).
    pub(super) resource_type: String,
    pub(super) type_index: usize,
    pub(super) type_total: u64,
    pub(super) type_elapsed: Duration,
    pub(super) type_counters: Counters,
    pub(super) processed: u64,
    pub(super) total: u64,
    pub(super) elapsed: Duration,
    pub(super) interval: Duration,
    pub(super) interval_resources: u64,
}

pub(super) struct TypeSummary {
    pub(super) resource_type: String,
    pub(super) outcome: &'static str,
    pub(super) type_index: usize,
    pub(super) type_total: u64,
    pub(super) type_elapsed: Duration,
    pub(super) elapsed: Duration,
    pub(super) counters: Counters,
    /// Write streams the type was walked with: 1 unless the source split it
    /// into id ranges (#1403).
    pub(super) streams: u32,
    /// Time spent planning the type's streams; zero when it was not planned
    /// (#1403).
    pub(super) plan: Duration,
}

pub(super) struct JobSummary {
    pub(super) outcome: &'static str,
    pub(super) types_done: usize,
    pub(super) types: usize,
    pub(super) total: u64,
    pub(super) elapsed: Duration,
    pub(super) counters: Counters,
}

struct OpenType {
    resource_type: String,
    type_index: usize,
    type_total: u64,
    started: Instant,
    counters: Counters,
    streams: u32,
    plan: Duration,
}

/// Local, single-job accounting: nothing here is a process-global, unlike
/// `crate::perf`'s counters, so concurrent jobs never mix.
pub(super) struct ReindexRunStats {
    started: Instant,
    total: u64,
    types: usize,
    progress_every: Duration,
    last_progress_at: Instant,
    last_progress_resources: u64,
    job: Counters,
    types_started: usize,
    types_done: usize,
    current: Option<OpenType>,
}

impl ReindexRunStats {
    pub(super) fn new(
        started: Instant,
        total: u64,
        types: usize,
        progress_every: Duration,
    ) -> Self {
        Self {
            started,
            total,
            types,
            progress_every,
            last_progress_at: started,
            last_progress_resources: 0,
            job: Counters::default(),
            types_started: 0,
            types_done: 0,
            current: None,
        }
    }

    pub(super) fn total(&self) -> u64 {
        self.total
    }

    pub(super) fn types(&self) -> usize {
        self.types
    }

    pub(super) fn mark_pages_started(&mut self, now: Instant) {
        self.last_progress_at = now;
        self.last_progress_resources = self.job.resources;
    }

    pub(super) fn start_type(
        &mut self,
        resource_type: &str,
        type_total: u64,
        now: Instant,
    ) -> TypeStarted {
        debug_assert!(
            self.current.is_none(),
            "a type must be finished before the next one starts"
        );
        self.types_started += 1;
        let type_index = self.types_started;
        self.current = Some(OpenType {
            resource_type: resource_type.to_string(),
            type_index,
            type_total,
            started: now,
            counters: Counters::default(),
            streams: 1,
            plan: Duration::ZERO,
        });
        TypeStarted {
            resource_type: resource_type.to_string(),
            type_index,
            types: self.types,
            type_total,
            elapsed: now.saturating_duration_since(self.started),
        }
    }

    pub(super) fn record_page(&mut self, page: PageRecord, now: Instant) -> RecordedPage {
        self.job.add_page(&page);
        let (page_number, type_elapsed) = if let Some(open) = self.current.as_mut() {
            open.counters.add_page(&page);
            (
                open.counters.pages,
                now.saturating_duration_since(open.started),
            )
        } else {
            (0, Duration::ZERO)
        };

        let progress =
            if now.saturating_duration_since(self.last_progress_at) >= self.progress_every {
                let interval = now.saturating_duration_since(self.last_progress_at);
                let interval_resources = self
                    .job
                    .resources
                    .saturating_sub(self.last_progress_resources);
                let (resource_type, type_index, type_total, type_elapsed_snap, type_counters) =
                    match &self.current {
                        Some(open) => (
                            open.resource_type.clone(),
                            open.type_index,
                            open.type_total,
                            now.saturating_duration_since(open.started),
                            open.counters,
                        ),
                        None => (
                            NO_TYPE.to_string(),
                            0,
                            0,
                            Duration::ZERO,
                            Counters::default(),
                        ),
                    };
                self.last_progress_at = now;
                self.last_progress_resources = self.job.resources;
                Some(ProgressSnapshot {
                    resource_type,
                    type_index,
                    type_total,
                    type_elapsed: type_elapsed_snap,
                    type_counters,
                    processed: self.job.resources,
                    total: self.total,
                    elapsed: now.saturating_duration_since(self.started),
                    interval,
                    interval_resources,
                })
            } else {
                None
            };

        RecordedPage {
            page: page_number,
            type_elapsed,
            progress,
        }
    }

    pub(super) fn add_yield(&mut self, d: Duration) {
        self.job.add_yield(d);
        if let Some(open) = self.current.as_mut() {
            open.counters.add_yield(d);
        }
    }

    /// Records how the open type is walked: its write streams and the time
    /// spent planning them (#1403). Does nothing when no type is open.
    pub(super) fn set_type_plan(&mut self, streams: u32, plan: Duration) {
        if let Some(open) = self.current.as_mut() {
            open.streams = streams;
            open.plan = plan;
        }
    }

    pub(super) fn finish_type(
        &mut self,
        outcome: &'static str,
        now: Instant,
    ) -> Option<TypeSummary> {
        let open = self.current.take()?;
        if outcome == OUTCOME_COMPLETED {
            self.types_done += 1;
        }
        Some(TypeSummary {
            resource_type: open.resource_type,
            outcome,
            type_index: open.type_index,
            type_total: open.type_total,
            type_elapsed: now.saturating_duration_since(open.started),
            elapsed: now.saturating_duration_since(self.started),
            counters: open.counters,
            streams: open.streams,
            plan: open.plan,
        })
    }

    pub(super) fn finish_job(&self, outcome: &'static str, now: Instant) -> JobSummary {
        JobSummary {
            outcome,
            types_done: self.types_done,
            types: self.types,
            total: self.total,
            elapsed: now.saturating_duration_since(self.started),
            counters: self.job,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_page_adds_to_the_open_type_and_the_job() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 10, 1, Duration::from_secs(60));
        stats.mark_pages_started(t0);
        stats.start_type("Patient", 10, t0);

        let page1 = PageRecord {
            resources: 3,
            entries: 3,
            fetch: Duration::from_millis(5),
            write: Duration::from_millis(7),
            ..Default::default()
        };
        let recorded1 = stats.record_page(page1, t0 + Duration::from_secs(1));
        assert_eq!(recorded1.page, 1);
        assert_eq!(recorded1.type_elapsed, Duration::from_secs(1));

        let page2 = PageRecord {
            resources: 4,
            entries: 4,
            fetch: Duration::from_millis(6),
            write: Duration::from_millis(8),
            ..Default::default()
        };
        let recorded2 = stats.record_page(page2, t0 + Duration::from_secs(2));
        assert_eq!(recorded2.page, 2);
        assert_eq!(recorded2.type_elapsed, Duration::from_secs(2));

        let summary = stats
            .finish_type(OUTCOME_COMPLETED, t0 + Duration::from_secs(2))
            .unwrap();
        assert_eq!(summary.counters.resources, 7);
        assert_eq!(summary.counters.entries, 7);
        assert_eq!(summary.counters.pages, 2);
        assert_eq!(summary.counters.fetch, Duration::from_millis(11));
        assert_eq!(summary.counters.write, Duration::from_millis(15));

        let job = stats.finish_job(OUTCOME_COMPLETED, t0 + Duration::from_secs(2));
        assert_eq!(job.counters.resources, 7);
        assert_eq!(job.counters.pages, 2);
    }

    #[test]
    fn finish_type_reports_both_clocks_and_closes_the_type() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 5, 1, Duration::from_secs(60));
        stats.mark_pages_started(t0 + Duration::from_secs(1));
        stats.start_type("Patient", 5, t0 + Duration::from_secs(1));
        let summary = stats
            .finish_type(OUTCOME_COMPLETED, t0 + Duration::from_secs(4))
            .unwrap();
        assert_eq!(summary.type_elapsed, Duration::from_secs(3));
        assert_eq!(summary.elapsed, Duration::from_secs(4));
        assert!(
            stats
                .finish_type(OUTCOME_COMPLETED, t0 + Duration::from_secs(5))
                .is_none()
        );
    }

    #[test]
    fn set_type_plan_is_reported_when_the_type_finishes() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 1, 1, Duration::from_secs(60));
        stats.set_type_plan(4, Duration::from_millis(7)); // no type open: ignored
        stats.start_type("Observation", 1, t0);
        stats.set_type_plan(3, Duration::from_millis(12));
        let summary = stats
            .finish_type(OUTCOME_COMPLETED, t0 + Duration::from_secs(1))
            .unwrap();
        assert_eq!(summary.streams, 3);
        assert_eq!(summary.plan, Duration::from_millis(12));
    }

    #[test]
    fn types_done_counts_only_completed_types() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 2, 2, Duration::from_secs(60));
        stats.mark_pages_started(t0);
        stats.start_type("Patient", 1, t0);
        stats.finish_type(OUTCOME_COMPLETED, t0 + Duration::from_secs(1));
        stats.start_type("Observation", 1, t0 + Duration::from_secs(1));
        stats.finish_type(OUTCOME_FAILED, t0 + Duration::from_secs(2));
        let job = stats.finish_job(OUTCOME_FAILED, t0 + Duration::from_secs(2));
        assert_eq!(job.types_done, 1);
    }

    #[test]
    fn progress_is_due_once_per_interval() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 100, 1, Duration::from_secs(60));
        stats.mark_pages_started(t0);
        stats.start_type("Patient", 100, t0);
        let rec = |n: u64| PageRecord {
            resources: n,
            ..Default::default()
        };

        assert!(
            stats
                .record_page(rec(10), t0 + Duration::from_secs(30))
                .progress
                .is_none()
        );

        let snap = stats
            .record_page(rec(10), t0 + Duration::from_secs(61))
            .progress
            .unwrap();
        assert_eq!(snap.interval, Duration::from_secs(61));
        assert_eq!(snap.interval_resources, 20);
        assert_eq!(snap.elapsed, Duration::from_secs(61));

        assert!(
            stats
                .record_page(rec(5), t0 + Duration::from_secs(62))
                .progress
                .is_none()
        );

        let snap2 = stats
            .record_page(rec(5), t0 + Duration::from_secs(122))
            .progress
            .unwrap();
        assert_eq!(snap2.interval, Duration::from_secs(61));
        assert_eq!(snap2.interval_resources, 10);
    }

    #[test]
    fn first_interval_starts_when_pages_start() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 100, 1, Duration::from_secs(60));
        stats.mark_pages_started(t0 + Duration::from_secs(90));
        stats.start_type("Patient", 100, t0 + Duration::from_secs(90));
        let rec = |n: u64| PageRecord {
            resources: n,
            ..Default::default()
        };

        assert!(
            stats
                .record_page(rec(1), t0 + Duration::from_secs(120))
                .progress
                .is_none()
        );

        let snap = stats
            .record_page(rec(1), t0 + Duration::from_secs(151))
            .progress
            .unwrap();
        assert_eq!(snap.interval, Duration::from_secs(61));
        assert_eq!(snap.elapsed, Duration::from_secs(151));
    }

    #[test]
    fn zero_interval_reports_after_every_page() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 10, 1, Duration::ZERO);
        stats.mark_pages_started(t0);
        stats.start_type("Patient", 10, t0);
        let rec = PageRecord {
            resources: 1,
            ..Default::default()
        };
        assert!(stats.record_page(rec, t0).progress.is_some());
        assert!(
            stats
                .record_page(rec, t0 + Duration::from_millis(1))
                .progress
                .is_some()
        );
    }

    #[test]
    fn progress_snapshot_is_scoped_to_the_open_type() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 9, 2, Duration::ZERO);
        stats.mark_pages_started(t0);
        stats.start_type("Patient", 5, t0);
        stats.record_page(
            PageRecord {
                resources: 2,
                ..Default::default()
            },
            t0,
        );
        stats.record_page(
            PageRecord {
                resources: 3,
                ..Default::default()
            },
            t0,
        );
        stats.finish_type(OUTCOME_COMPLETED, t0);

        stats.start_type("Observation", 4, t0);
        let snap = stats
            .record_page(
                PageRecord {
                    resources: 4,
                    ..Default::default()
                },
                t0,
            )
            .progress
            .unwrap();
        assert_eq!(snap.resource_type, "Observation");
        assert_eq!(snap.type_index, 2);
        assert_eq!(snap.type_counters.pages, 1);
        assert_eq!(snap.type_counters.resources, 4);
        assert_eq!(snap.processed, 9);

        stats.finish_type(OUTCOME_COMPLETED, t0);
        let recorded = stats.record_page(
            PageRecord {
                resources: 1,
                ..Default::default()
            },
            t0,
        );
        assert_eq!(recorded.page, 0);
        assert_eq!(recorded.type_elapsed, Duration::ZERO);
        let snap2 = recorded.progress.unwrap();
        assert_eq!(snap2.resource_type, NO_TYPE);
        assert_eq!(snap2.type_counters, Counters::default());
        assert_eq!(snap2.processed, 10);
    }

    #[test]
    fn phase_millis_subtract_before_truncating_and_saturate() {
        let mut c = Counters {
            write: Duration::from_millis(10),
            ..Default::default()
        };
        c.writer.extract = Duration::from_millis(3);
        c.writer.delete = Duration::from_millis(2);
        c.writer.insert = Duration::from_millis(4);
        assert_eq!(
            PhaseMillis::of(&c, Duration::from_millis(10)).writer_other_ms,
            1
        );

        let mut c2 = Counters {
            write: Duration::from_millis(5),
            ..Default::default()
        };
        c2.writer.extract = Duration::from_millis(10);
        assert_eq!(
            PhaseMillis::of(&c2, Duration::from_millis(5)).writer_other_ms,
            0
        );

        let c3 = Counters {
            fetch: Duration::from_millis(20),
            write: Duration::from_millis(20),
            yielded: Duration::from_millis(20),
            ..Default::default()
        };
        assert_eq!(PhaseMillis::of(&c3, Duration::from_millis(10)).other_ms, 0);

        let mut c4 = Counters {
            write: Duration::from_nanos(2_500_000),
            ..Default::default()
        };
        c4.writer.extract = Duration::from_nanos(1_600_000);
        assert_eq!(
            PhaseMillis::of(&c4, Duration::from_nanos(2_500_000)).writer_other_ms,
            0,
            "truncating extract and write separately before subtracting would wrongly give 1"
        );

        let c5 = Counters {
            fetch: Duration::from_nanos(1_999_999),
            ..Default::default()
        };
        assert_eq!(
            PhaseMillis::of(&c5, Duration::from_nanos(1_999_999)).fetch_ms,
            1
        );
    }

    #[test]
    fn phase_millis_uses_the_critical_path() {
        let mut c = Counters::default();
        c.write = Duration::from_millis(100);
        c.fetch = Duration::from_millis(20);
        c.fetch_wait = Duration::from_millis(5); // most of the fetch was hidden by prefetch
        c.yielded = Duration::from_millis(5);
        c.writer.extract = Duration::from_millis(30);
        c.writer.delete = Duration::from_millis(10);
        c.writer.insert = Duration::from_millis(40);
        // db_wait is unset: db_wait_or_busy() falls back to delete + insert = 50 ms.
        let m = PhaseMillis::of(&c, Duration::from_millis(130));
        assert_eq!(m.fetch_ms, 20);
        assert_eq!(m.fetch_wait_ms, 5);
        assert_eq!(m.db_wait_ms, 50);
        assert_eq!(
            m.writer_other_ms, 20,
            "write - (extract 30 + db_wait_or_busy 50) = 20"
        );
        assert_eq!(
            m.other_ms, 20,
            "elapsed 130 - (fetch_wait 5 + write 100 + yield 5) = 20"
        );
    }

    #[test]
    fn unmeasured_db_wait_keeps_the_busy_formula() {
        // A writer that never sets db_wait (every non-MongoDB writer today)
        // must give the same writer_other_ms as delete + insert would.
        let mut c = Counters::default();
        c.write = Duration::from_millis(10);
        c.writer.extract = Duration::from_millis(3);
        c.writer.delete = Duration::from_millis(2);
        c.writer.insert = Duration::from_millis(4);
        let m = PhaseMillis::of(&c, Duration::from_millis(10));
        assert_eq!(m.writer_other_ms, 1);
    }

    #[test]
    fn unprefetched_pages_keep_fetch_in_other_ms() {
        // fetch_wait == fetch (nothing prefetched) must give the same other_ms
        // as computing it from fetch directly.
        let mut c = Counters::default();
        c.fetch = Duration::from_millis(7);
        c.fetch_wait = Duration::from_millis(7);
        c.write = Duration::from_millis(3);
        c.yielded = Duration::from_millis(1);
        let m = PhaseMillis::of(&c, Duration::from_millis(20));
        assert_eq!(m.other_ms, 9);
    }

    #[test]
    fn record_page_accumulates_fetch_wait_and_db_wait() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 10, 1, Duration::from_secs(60));
        stats.mark_pages_started(t0);
        let _ = stats.start_type("Patient", 10, t0);
        let page = PageRecord {
            resources: 5,
            entries: 5,
            failed: 0,
            fetch: Duration::from_millis(50),
            fetch_wait: Duration::from_millis(10),
            write: Duration::from_millis(200),
            writer: ReindexPageStats {
                db_wait: Some(Duration::from_millis(150)),
                ..ReindexPageStats::default()
            },
        };
        let recorded = stats.record_page(page, t0 + Duration::from_millis(300));
        assert_eq!(recorded.page, 1);
        let summary = stats
            .finish_type(OUTCOME_COMPLETED, t0 + Duration::from_millis(300))
            .expect("a type is open");
        assert_eq!(summary.counters.fetch_wait, Duration::from_millis(10));
        assert_eq!(
            summary.counters.writer.db_wait,
            Some(Duration::from_millis(150))
        );
    }

    #[test]
    fn per_second_rounds_and_never_divides_by_zero() {
        assert_eq!(per_second(3, Duration::from_secs(2)), 1.5);
        assert_eq!(per_second(1000, Duration::ZERO), 0.0);
        assert_eq!(per_second(2, Duration::from_secs(3)), 0.7);
        assert_eq!(per_second(1, Duration::from_secs(3)), 0.3);
    }

    #[test]
    fn yield_adds_to_the_type_and_the_job() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 1, 1, Duration::from_secs(60));
        stats.mark_pages_started(t0);
        stats.start_type("Patient", 1, t0);
        stats.add_yield(Duration::from_millis(5));
        let summary = stats.finish_type(OUTCOME_COMPLETED, t0).unwrap();
        assert_eq!(summary.counters.yielded, Duration::from_millis(5));
        let job = stats.finish_job(OUTCOME_COMPLETED, t0);
        assert_eq!(job.counters.yielded, Duration::from_millis(5));
    }

    #[test]
    fn a_type_reports_one_stream_and_no_plan_time_by_default() {
        let t0 = Instant::now();
        let mut stats = ReindexRunStats::new(t0, 1, 1, Duration::from_secs(60));
        stats.start_type("Patient", 1, t0);
        let summary = stats
            .finish_type(OUTCOME_COMPLETED, t0 + Duration::from_secs(1))
            .unwrap();
        assert_eq!(summary.streams, 1);
        assert_eq!(summary.plan, Duration::ZERO);
    }
}
