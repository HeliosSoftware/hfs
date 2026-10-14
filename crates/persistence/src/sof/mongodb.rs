//! MongoDB in-DB SQL-on-FHIR runner.
//!
//! [`MongoInDbRunner`] compiles a ViewDefinition to a MongoDB aggregation
//! pipeline (via [`compile_view_definition_mongo`](super::compiler::compile_view_definition_mongo))
//! and executes it directly against the `resources` collection, bypassing
//! in-process FHIRPath evaluation. Resources are stored as native BSON
//! sub-documents under `data`, so the emitted pipeline navigates them with
//! `$getField`/dotted paths.
//!
//! The runtime filters that depend on the request (tenant, `_since`, row limit)
//! are layered onto the compiled pipeline here: a leading `$match` constrains
//! the tenant (and `last_updated` for `since`), and a trailing `$limit` caps the
//! output.
//!
//! The `aggregate` and its cursor loop run in a supervised task that sends the
//! rows through the producer core's batch channel
//! ([`row_batch_channel`](crate::core::sof_producer::row_batch_channel)), so a
//! panic reaches the consumer as an `Err` item and a partial row set never
//! looks complete.
//!
//! # Stopping a run
//!
//! A run stops when its stream is dropped, on [`RunContext::cancel`], and at
//! [`RunContext::deadline`]. Every `aggregate` carries the options of
//! [`aggregate_options`]: a unique `comment` and `maxTimeMS` from the time
//! left. The initial command and each cursor step are raced against the
//! stop:
//!
//! - while the cursor is open, the cursor is dropped, and the driver kills it
//!   on the server;
//! - while the initial `aggregate` has not returned (a `$facet` or `$sort`
//!   reads the whole type before its first batch), its future is dropped, so
//!   the driver closes the connection and the server kills the read of a
//!   disconnected client (MongoDB 4.2+); a cleanup task also finds the command
//!   by its comment through `$currentOp` (own operations) and kills it with
//!   `killOp`, under a short per-command timeout.
//!
//! `maxTimeMS` stays the server-side ceiling. A deployment that refuses both
//! the own-operation `killOp` and the disconnect kill is bounded by it, that
//! is by the run's deadline: a stated exception to the stop bound.

use std::future::IntoFuture;
use std::sync::Arc;
use std::time::Duration;

use mongodb::bson::{Bson, DateTime as BsonDateTime, Document, doc};
use mongodb::options::AggregateOptions;
use mongodb::{Client, Collection};
use tokio::sync::OnceCell;
use tracing::{debug, warn};

use crate::backends::mongodb::backend::{MongoBackend, MongoBackendConfig, connect_client};
use crate::core::sof_producer::{
    BATCH_LINGER, BatchSender, RunStop, row_batch_channel, spawn_supervised,
};
use crate::core::sof_runner::{
    RunCapabilities, RunContext, RunOutput, RunRequest, SofError, SofRunner, StopReason, ViewRow,
};
use crate::core::sof_schema::ViewSchema;
use crate::tenant::TenantContext;

use super::compiler::compile_view_definition_mongo;

/// The runner's name in logs.
const RUNNER: &str = "mongo-indb";

/// Prefix of the unique `comment` every SoF `aggregate` carries.
const RUN_COMMENT_PREFIX: &str = "hfs-sof-run-";

/// How long after the deadline the client stops a step the server bounds
/// with `maxTimeMS`: a backstop, so a run that times out on the server keeps
/// its connection.
const DEADLINE_GRACE: Duration = Duration::from_secs(1);

/// How often a stop's cleanup looks for the initial `aggregate` in
/// `$currentOp`, and for how long: the command may register a moment after
/// the stop.
const CURRENT_OP_POLL: Duration = Duration::from_millis(20);
const CURRENT_OP_WAIT: Duration = Duration::from_millis(500);

/// The time limit of each cleanup command (`$currentOp`, `killOp`), so a
/// saturated pool cannot hold the cleanup.
const CLEANUP_COMMAND_TIMEOUT: Duration = Duration::from_millis(300);

/// SQL-on-FHIR runner that compiles ViewDefinitions to MongoDB aggregation
/// pipelines.
pub struct MongoInDbRunner {
    /// Shared (lazily initialised) client cell — the same pool the backend uses.
    client: Arc<OnceCell<Client>>,
    config: MongoBackendConfig,
    /// Where a run with `patient`/`group` filters goes: the pipeline compiler
    /// has no compartment predicate yet, so those runs are handed to another
    /// runner (the backend wires the in-process engine over a scan of the
    /// same collection) instead of failing as uncompilable.
    compartment_fallback: Option<Arc<dyn SofRunner>>,
}

impl MongoInDbRunner {
    /// Creates a runner sharing the backend's client cell and configuration.
    pub fn new(client: Arc<OnceCell<Client>>, config: MongoBackendConfig) -> Self {
        Self {
            client,
            config,
            compartment_fallback: None,
        }
    }

    /// Routes runs that carry `patient`/`group` filters to `runner`. Without
    /// one, such a run is refused as uncompilable.
    pub fn with_compartment_fallback(mut self, runner: Arc<dyn SofRunner>) -> Self {
        self.compartment_fallback = Some(runner);
        self
    }

    /// Resolves the `resources` collection, initialising the shared client on
    /// first use.
    async fn resources(&self) -> Result<Collection<Document>, SofError> {
        let client = self
            .client
            .get_or_try_init(|| connect_client(&self.config))
            .await
            .map_err(|e| SofError::Storage(e.to_string()))?;
        Ok(client
            .database(&self.config.database_name)
            .collection(MongoBackend::RESOURCES_COLLECTION))
    }
}

#[async_trait::async_trait]
impl SofRunner for MongoInDbRunner {
    fn runner_name(&self) -> &'static str {
        RUNNER
    }

    async fn run_view(
        &self,
        tenant: &TenantContext,
        request: RunRequest,
        ctx: RunContext,
    ) -> Result<RunOutput, SofError> {
        if !request.filters.patient.is_empty() || !request.filters.group.is_empty() {
            // Compartment filtering has no pipeline form yet: hand the run to
            // the fallback runner when the backend wired one.
            return match &self.compartment_fallback {
                Some(runner) => {
                    debug!(
                        runner = runner.runner_name(),
                        tenant = %tenant.tenant_id(),
                        "patient/group filters: delegating the view run"
                    );
                    runner.run_view(tenant, request, ctx).await
                }
                None => Err(SofError::Uncompilable {
                    reason: "patient/group filters are not yet supported by the MongoDB runner"
                        .to_string(),
                }),
            };
        }

        let RunRequest {
            view: view_definition,
            filters,
        } = request;
        let compiled = compile_view_definition_mongo(&view_definition, self.config.fhir_version)?;
        let schema = ViewSchema::from_view_definition(&view_definition);
        // Once per run: the pipeline's `$project` order into document order.
        let permutation = schema.permutation_from(&compiled.columns)?;

        debug!(
            runner = RUNNER,
            tenant = %tenant.tenant_id(),
            "executing compiled ViewDefinition"
        );

        // Leading tenant (and optional `since`) predicate, prepended to the
        // compiled pipeline. Kept first so the `tenant_id + resource_type +
        // is_deleted` index can serve the scan.
        let mut tenant_match = doc! { "tenant_id": tenant.tenant_id().to_string() };
        if let Some(since) = filters.since {
            // MongoDB stores milliseconds: a finer cutoff rounds up to the
            // millisecond (#1864), so one a fraction of a millisecond after a
            // resource's stored stamp drops it instead of keeping it (the old
            // floor did). A cutoff equal to the stamp still keeps it.
            tenant_match.insert(
                "last_updated",
                doc! { "$gte": BsonDateTime::from_millis(ceil_to_millis(since).timestamp_millis()) },
            );
        }

        let mut pipeline: Vec<Document> = Vec::with_capacity(compiled.pipeline.len() + 2);
        pipeline.push(doc! { "$match": tenant_match });
        pipeline.extend(compiled.pipeline);
        if let Some(limit) = filters.limit {
            pipeline.push(doc! { "$limit": limit as i64 });
        }

        let collection = race_setup(&ctx, self.resources()).await?;
        let (out, stop, rows) = row_batch_channel(RUNNER, &ctx);
        let run = AggregateRun {
            comment: format!("{RUN_COMMENT_PREFIX}{}", uuid::Uuid::new_v4().simple()),
            collection,
            pipeline,
        };
        let options = aggregate_options(&run.comment, stop.remaining());
        let reporter = out.reporter();
        spawn_supervised(RUNNER, reporter, produce(run, options, stop, out));

        // The pipeline does not sort, so its rows have no total order. A stop
        // ends the server's work: the cursor is killed, and an initial
        // `aggregate` that has not returned yet is killed by its comment and
        // by the closed connection (see `produce`).
        let capabilities = RunCapabilities {
            total_order: false,
            declared_schema: true,
            cancels_server_side: true,
        };
        Ok(RunOutput::new(
            schema,
            capabilities,
            permutation.normalize(rows),
        ))
    }
}

/// Runs a setup step of a run (connecting the shared client) unless the run
/// is cancelled or its deadline passes first. A run whose deadline has
/// already passed ends here, before any command.
async fn race_setup<T>(
    ctx: &RunContext,
    setup: impl std::future::Future<Output = Result<T, SofError>>,
) -> Result<T, SofError> {
    let stop = RunStop::new(ctx);
    if let Some(reason) = stop.check() {
        return Err(stop.error_for(reason).unwrap_or(SofError::Cancelled));
    }
    tokio::select! {
        biased;
        reason = stop.stopped() => Err(stop.error_for(reason).unwrap_or(SofError::Cancelled)),
        result = setup => result,
    }
}

/// The options of every SoF `aggregate`: the one place they are built.
///
/// - `comment`: the run's unique comment. The server echoes it in
///   `$currentOp`, which is how a stop finds the initial command to kill.
/// - `max_time` (`maxTimeMS`): the time left before the run's deadline,
///   rounded up to whole milliseconds and at least 1 ms (a `0` would mean
///   "no limit"). It is the server-side ceiling of the run, not how a stop
///   ends the work.
pub(crate) fn aggregate_options(comment: &str, remaining: Option<Duration>) -> AggregateOptions {
    AggregateOptions::builder()
        .comment(Bson::String(comment.to_string()))
        .max_time(remaining.map(max_time))
        .build()
}

/// `t` rounded up to the next whole millisecond (unchanged when it already is
/// one), the precision MongoDB stores a date with. The `_since` rule: a backend
/// that stores a coarser instant than the cutoff rounds the cutoff **up** to its
/// precision, so `_since` stays inclusive at the stored precision (#1864).
/// Saturates at `t` itself on overflow, which no stored instant reaches.
pub(crate) fn ceil_to_millis(t: chrono::DateTime<chrono::Utc>) -> chrono::DateTime<chrono::Utc> {
    let sub_milli = t.timestamp_subsec_nanos() % 1_000_000;
    if sub_milli == 0 {
        return t;
    }
    t.checked_add_signed(chrono::Duration::nanoseconds(i64::from(
        1_000_000 - sub_milli,
    )))
    .unwrap_or(t)
}

/// `remaining` as a `maxTimeMS`: rounded up to whole milliseconds, at least
/// 1 ms.
fn max_time(remaining: Duration) -> Duration {
    let ms = remaining.as_nanos().div_ceil(1_000_000).max(1);
    Duration::from_millis(u64::try_from(ms).unwrap_or(u64::MAX))
}

/// One run's `aggregate`: its collection, pipeline and unique comment.
struct AggregateRun {
    comment: String,
    collection: Collection<Document>,
    pipeline: Vec<Document>,
}

/// The run's producer: the initial `aggregate`, then the cursor, each step
/// raced against the stop.
///
/// - A stop while the initial `aggregate` has not returned drops its future.
///   The driver never re-pools a connection with a command in flight, so it
///   closes that connection, and MongoDB (4.2+) kills a read operation whose
///   client disconnected. In parallel, [`kill_initial_command`] looks the
///   command up by its comment and kills it, for the case the disconnect is
///   not seen in time.
/// - A stop while the cursor is open drops the cursor, which makes the
///   driver kill it on the server (`killCursors`).
/// - The deadline: the server ends the work at `maxTimeMS`; the client acts on
///   the deadline only [`DEADLINE_GRACE`] later, as a backstop, so a
///   timed-out run does not also close its connection. Between steps the
///   deadline is checked without grace.
async fn produce(
    run: AggregateRun,
    options: AggregateOptions,
    stop: RunStop,
    mut out: BatchSender,
) {
    let AggregateRun {
        comment,
        collection,
        pipeline,
    } = run;
    if let Some(reason) = stop.check() {
        return out.end_after_stop(&stop, reason).await;
    }
    let started = {
        let aggregate = collection.aggregate(pipeline).with_options(options);
        tokio::select! {
            biased;
            reason = stop.stopped_with_grace(DEADLINE_GRACE) => Err(reason),
            result = aggregate.into_future() => Ok(result),
        }
        // A stop drops the in-flight `aggregate` future here.
    };
    let mut cursor = match started {
        Ok(Ok(cursor)) => cursor,
        Ok(Err(e)) => {
            return out
                .fail(&stop, run_error(&stop, "aggregate failed", e))
                .await;
        }
        Err(reason) => {
            tokio::spawn(kill_initial_command(collection.client().clone(), comment));
            debug!(
                runner = RUNNER,
                ?reason,
                "stopped during the initial aggregate"
            );
            return out.end_after_stop(&stop, reason).await;
        }
    };

    let mut count = 0usize;
    loop {
        if let Some(reason) = stop.check() {
            drop(cursor);
            return out.end_after_stop(&stop, reason).await;
        }
        let advanced = {
            let next = cursor.advance();
            tokio::pin!(next);
            loop {
                // A partial batch waits at most BATCH_LINGER for more rows
                // while the cursor fetches its next batch.
                let linger = out
                    .pending_age()
                    .map(|age| BATCH_LINGER.saturating_sub(age));
                tokio::select! {
                    biased;
                    reason = stop.stopped_with_grace(DEADLINE_GRACE) => break Err(reason),
                    advanced = &mut next => break Ok(advanced),
                    _ = async move {
                        match linger {
                            Some(wait) => tokio::time::sleep(wait).await,
                            None => std::future::pending().await,
                        }
                    } => {
                        if let Err(reason) = out.flush(&stop).await {
                            break Err(reason);
                        }
                    }
                }
            }
        };
        match advanced {
            Ok(Ok(true)) => {}
            Ok(Ok(false)) => break,
            Ok(Err(e)) => return out.fail(&stop, run_error(&stop, "cursor advance", e)).await,
            Err(reason) => {
                // Dropping the cursor kills it on the server.
                drop(cursor);
                debug!(
                    runner = RUNNER,
                    ?reason,
                    rows = count,
                    "stopped while reading the cursor"
                );
                return out.end_after_stop(&stop, reason).await;
            }
        }

        let row = match cursor.deserialize_current() {
            Ok(doc) => match mongodb::bson::from_bson::<ViewRow>(Bson::Document(doc)) {
                Ok(row) => row,
                Err(e) => {
                    return out
                        .fail(&stop, SofError::backend(format!("row decode: {e}")))
                        .await;
                }
            },
            Err(e) => {
                return out
                    .fail(&stop, SofError::backend(format!("cursor deserialize: {e}")))
                    .await;
            }
        };
        count += 1;
        if out.push(Ok(row)) {
            if let Err(reason) = out.flush(&stop).await {
                drop(cursor);
                return out.end_after_stop(&stop, reason).await;
            }
        }
    }

    debug!(runner = RUNNER, rows = count, "in-DB view run complete");
    out.finish(&stop).await;
}

/// A driver error of the run, with its kind. A server time limit
/// (`maxTimeMS`) that fired because the run's deadline passed is the run's
/// timeout, carrying its budget.
fn run_error(stop: &RunStop, context: &str, e: mongodb::error::Error) -> SofError {
    let error = SofError::from_backend_error(crate::error::classify_mongodb_error(context, e));
    match error {
        SofError::Timeout { .. } if stop.deadline_passed() => stop
            .error_for(StopReason::DeadlineExceeded)
            .unwrap_or(error),
        other => other,
    }
}

/// Kills the initial `aggregate` of a stopped run, found by its comment.
///
/// The command may not be registered yet when the stop arrives, so this
/// polls `$currentOp` (own operations only, `allUsers: false`, which needs no
/// `inprog` privilege) every [`CURRENT_OP_POLL`] for at most
/// [`CURRENT_OP_WAIT`], and issues `killOp` (allowed on one's own operations
/// without the `killop` privilege) for what it finds. Each command runs under
/// [`CLEANUP_COMMAND_TIMEOUT`] (`maxTimeMS` where the command takes it, and a
/// client timeout), so a saturated pool cannot hold the cleanup: on a timeout
/// or an error it gives up with a warning, and the closed connection is what
/// ends the operation.
async fn kill_initial_command(client: Client, comment: String) {
    let admin = client.database("admin");
    let started = tokio::time::Instant::now();
    loop {
        let lookup = async {
            let mut cursor = admin
                .aggregate(vec![
                    doc! {"$currentOp": {"allUsers": false, "idleConnections": false}},
                    doc! {"$match": {"command.comment": &comment}},
                    doc! {"$project": {"opid": 1}},
                ])
                .max_time(CLEANUP_COMMAND_TIMEOUT)
                .await?;
            let mut opids = Vec::new();
            while cursor.advance().await? {
                if let Some(opid) = cursor.deserialize_current()?.get("opid") {
                    opids.push(opid.clone());
                }
            }
            Ok::<_, mongodb::error::Error>(opids)
        };
        let opids = match tokio::time::timeout(CLEANUP_COMMAND_TIMEOUT, lookup).await {
            Ok(Ok(opids)) => opids,
            Ok(Err(e)) => {
                warn!(runner = RUNNER, %comment, error = %e,
                    "could not look up the stopped aggregate; the closed connection ends it");
                return;
            }
            Err(_) => {
                warn!(runner = RUNNER, %comment,
                    "looking up the stopped aggregate timed out; the closed connection ends it");
                return;
            }
        };
        if !opids.is_empty() {
            for opid in opids {
                let kill = admin.run_command(doc! {"killOp": 1, "op": opid});
                match tokio::time::timeout(CLEANUP_COMMAND_TIMEOUT, kill).await {
                    Ok(Ok(_)) => {
                        debug!(runner = RUNNER, %comment, "killed the stopped aggregate");
                    }
                    Ok(Err(e)) => warn!(runner = RUNNER, %comment, error = %e,
                        "killOp of the stopped aggregate failed; the closed connection ends it"),
                    Err(_) => warn!(runner = RUNNER, %comment,
                        "killOp of the stopped aggregate timed out; the closed connection ends it"),
                }
            }
            return;
        }
        if started.elapsed() >= CURRENT_OP_WAIT {
            // Never registered, or already ended by the closed connection.
            return;
        }
        tokio::time::sleep(CURRENT_OP_POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::ceil_to_millis;

    /// #1864 (A18): a `_since` cutoff finer than a millisecond rounds up to the
    /// next millisecond; a whole-millisecond one is unchanged.
    #[test]
    fn ceil_to_millis_rounds_sub_millisecond_cutoffs_up() {
        for (since, expected) in [
            (
                "2024-05-01T12:00:00.123000001Z",
                "2024-05-01T12:00:00.124000000Z",
            ),
            (
                "2024-05-01T12:00:00.123500000Z",
                "2024-05-01T12:00:00.124000000Z",
            ),
            (
                "2024-05-01T12:00:59.999000001Z",
                "2024-05-01T12:01:00.000000000Z",
            ),
            (
                "2024-05-01T12:00:00.123000000Z",
                "2024-05-01T12:00:00.123000000Z",
            ),
            ("2024-05-01T12:00:00Z", "2024-05-01T12:00:00.000000000Z"),
        ] {
            let since: chrono::DateTime<chrono::Utc> = since.parse().unwrap();
            assert_eq!(
                ceil_to_millis(since).to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
                expected,
                "{since}"
            );
        }
    }
}
