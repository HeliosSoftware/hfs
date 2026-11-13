//! Adversarial tests for the pipelined stream driver (review of the combined perf stack).
//!
//! A model engine (writes first, then charges lines above a file watermark in
//! its turn — the MongoDB / PostgreSQL shape) is driven with random batch
//! delays, faults (error / panic before, inside and after the turn), crashes
//! (the driver future dropped mid-file) and resumes, and the result of the
//! pipelined driver is compared with a port of the serial loop it replaced.

use super::*;
use crate::core::bulk_submit::CancelToken;
use crate::tenant::{TenantId, TenantPermissions};
use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::Duration;

fn tenant() -> TenantContext {
    TenantContext::new(TenantId::new("review"), TenantPermissions::full_access())
}

/// xorshift64*, deterministic per seed.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    ErrBeforeTurn,
    PanicBeforeTurn,
    PanicInsideTurn,
    ErrAfterCharge,
    PanicAfterCharge,
}

#[derive(Default, Debug)]
struct Model {
    /// File watermark (highest line charged).
    watermark: u64,
    /// Data lines a batch has charged (counted), across runs.
    charged_lines: BTreeSet<u64>,
    /// Lines counted twice (double charge) — must stay empty.
    double_counted: Vec<u64>,
    /// Lines a batch carried that were already under the watermark but
    /// never charged — a skipped, uncommitted line. Must stay empty.
    skipped: Vec<u64>,
    /// Seqs that charged in this run, in charge order.
    charged_seqs: Vec<u64>,
    /// Seqs whose writes landed (charged or not).
    written_seqs: Vec<u64>,
    /// Batches whose writes started while a cancel token was already set.
    started_after_cancel: usize,
}

#[derive(Clone)]
struct ModelWriter {
    cap: u32,
    model: Arc<Mutex<Model>>,
    delays: Arc<Vec<u64>>,
    fault: Option<(u64, Fault)>,
    /// Lines whose entry result is a processing error.
    fail_lines: Arc<HashSet<u64>>,
    /// Batch seq that sleeps this long after completing its turn.
    stall_after_complete: Option<(u64, Duration)>,
    cancel_at: Option<(u64, CancelToken)>,
}

impl ModelWriter {
    fn new(cap: u32, model: Arc<Mutex<Model>>) -> Self {
        Self {
            cap,
            model,
            delays: Arc::new(Vec::new()),
            fault: None,
            fail_lines: Arc::new(HashSet::new()),
            stall_after_complete: None,
            cancel_at: None,
        }
    }
}

#[async_trait]
impl StreamBatchWriter for ModelWriter {
    fn batch_writer_cap(&self) -> u32 {
        self.cap
    }

    async fn write_batch(
        &self,
        _tenant: &TenantContext,
        _submission_id: &SubmissionId,
        _manifest_id: &str,
        entries: Vec<NdjsonEntry>,
        options: &BulkProcessingOptions,
        ticket: BatchTicket,
    ) -> StorageResult<Vec<BulkEntryResult>> {
        let seq = ticket.seq();
        if options.is_cancelled() {
            self.model.lock().unwrap().started_after_cancel += 1;
        }
        // Cancel before `began`: the next batch is admitted as soon as this
        // one has begun, so cancelling after would let it slip in between
        // and be counted as started after the cancel.
        if let Some((at, token)) = &self.cancel_at
            && *at == seq
        {
            token.cancel();
        }
        ticket.began();
        let micros = self.delays.get(seq as usize).copied().unwrap_or(0);
        if micros > 0 {
            tokio::time::sleep(Duration::from_micros(micros)).await;
        } else {
            tokio::task::yield_now().await;
        }
        let results: Vec<BulkEntryResult> = entries
            .iter()
            .map(|e| {
                if self.fail_lines.contains(&e.line_number) {
                    BulkEntryResult::processing_error(
                        e.line_number,
                        &e.resource_type,
                        serde_json::json!({"resourceType": "OperationOutcome"}),
                    )
                } else {
                    BulkEntryResult::success(
                        e.line_number,
                        &e.resource_type,
                        e.resource_id.clone().unwrap_or_default(),
                        true,
                    )
                }
            })
            .collect();
        self.model.lock().unwrap().written_seqs.push(seq);
        let fault = self.fault.filter(|(at, _)| *at == seq).map(|(_, f)| f);
        match fault {
            Some(Fault::ErrBeforeTurn) => {
                return Err(internal_error(format!("batch {seq} failed")));
            }
            Some(Fault::PanicBeforeTurn) => panic!("batch {seq} panicked before its turn"),
            _ => {}
        }
        let turn = ticket.wait_charge().await?;
        // The charge statements: an await in the middle, as a real engine has.
        tokio::task::yield_now().await;
        if fault == Some(Fault::PanicInsideTurn) {
            panic!("batch {seq} panicked inside its turn, before the charge landed");
        }
        {
            let mut m = self.model.lock().unwrap();
            let wm = m.watermark;
            for e in &entries {
                if e.line_number > wm {
                    if !m.charged_lines.insert(e.line_number) {
                        m.double_counted.push(e.line_number);
                    }
                } else if !m.charged_lines.contains(&e.line_number) {
                    m.skipped.push(e.line_number);
                }
            }
            let last = entries.iter().map(|e| e.line_number).max().unwrap_or(0);
            m.watermark = m.watermark.max(last);
            m.charged_seqs.push(seq);
        }
        turn.complete();
        match fault {
            Some(Fault::ErrAfterCharge) => {
                return Err(internal_error(format!("batch {seq} failed after charge")));
            }
            Some(Fault::PanicAfterCharge) => panic!("batch {seq} panicked after its charge"),
            _ => {}
        }
        if let Some((at, stall)) = self.stall_after_complete
            && at == seq
        {
            tokio::time::sleep(stall).await;
        }
        Ok(results)
    }
}

/// A generated file: its text and which lines are data lines (reach a batch).
struct File {
    text: String,
    data_lines: BTreeSet<u64>,
}

fn gen_file(rng: &mut Rng, lines: u64, with_rejects: bool, dup_ids: bool) -> File {
    let mut text = String::new();
    let mut data_lines = BTreeSet::new();
    for line in 1..=lines {
        let roll = rng.below(20);
        if with_rejects && roll == 0 {
            text.push_str("{not json\n");
        } else if with_rejects && roll == 1 {
            text.push_str(&format!(
                "{{\"resourceType\":\"Observation\",\"id\":\"o{line}\"}}\n"
            ));
        } else if roll == 2 {
            text.push('\n');
        } else {
            let id = if dup_ids {
                format!("p{}", rng.below(6))
            } else {
                format!("p{line}")
            };
            text.push_str(&format!(
                "{{\"resourceType\":\"Patient\",\"id\":\"{id}\"}}\n"
            ));
            data_lines.insert(line);
        }
    }
    File { text, data_lines }
}

fn reader(text: &str) -> Box<dyn AsyncBufRead + Send + Unpin> {
    Box::new(tokio::io::BufReader::new(std::io::Cursor::new(
        text.as_bytes().to_vec(),
    )))
}

async fn drive_new(
    writer: ModelWriter,
    text: &str,
    options: &BulkProcessingOptions,
) -> StorageResult<StreamProcessingResult> {
    drive_ndjson_stream(
        writer,
        &tenant(),
        &SubmissionId::generate("review"),
        "m1",
        "Patient",
        reader(text),
        options,
    )
    .await
}

/// Port of the serial loop every engine carried before the driver
/// (postgres/bulk_submit.rs on perf/bsp-harness), over the same writer.
async fn drive_serial(
    writer: &ModelWriter,
    text: &str,
    options: &BulkProcessingOptions,
) -> StorageResult<StreamProcessingResult> {
    let tenant = tenant();
    let sub = SubmissionId::generate("review");
    let mut reader = reader(text);
    let mut result = StreamProcessingResult::new();
    let mut line_number = 0u64;
    let mut batch = Vec::new();
    let resource_type = "Patient";
    let mut seq = 0u64;
    if options.is_cancelled() {
        return Ok(result.aborted(CANCELLED_ABORT_REASON));
    }
    loop {
        let mut line = String::new();
        let bytes_read = reader.read_line(&mut line).await.unwrap();
        if bytes_read == 0 {
            break;
        }
        line_number += 1;
        result.lines_processed = line_number;
        if line_number <= options.resume_after_line {
            continue;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match NdjsonEntry::parse(line_number, line) {
            Ok(entry) => {
                if entry.resource_type != resource_type {
                    result.counts.increment(BulkEntryOutcome::ValidationError);
                    result.unbatched_errors += 1;
                    if !options.continue_on_error
                        && (options.max_errors == 0
                            || result.counts.error_count() >= options.max_errors as u64)
                    {
                        return Ok(result.aborted("max errors exceeded"));
                    }
                    continue;
                }
                batch.push(entry);
            }
            Err(e) => {
                result.counts.increment(BulkEntryOutcome::ValidationError);
                result.unbatched_errors += 1;
                if !options.continue_on_error
                    && (options.max_errors == 0
                        || result.counts.error_count() >= options.max_errors as u64)
                {
                    return Ok(result.aborted(format!("Parse error: {}", e)));
                }
            }
        }
        if batch.len() >= options.batch_size as usize {
            let seq_now = seq;
            seq += 1;
            let seqr = BatchSequencer::new();
            let _ = seq_now;
            let ticket = seqr.ticket(0);
            // Re-number via a dedicated single-batch sequence so the model's
            // seq-indexed scripts still line up.
            let results = writer
                .write_batch_seq(
                    &tenant,
                    &sub,
                    std::mem::take(&mut batch),
                    options,
                    ticket,
                    seq_now,
                )
                .await?;
            for r in results {
                result.counts.increment(r.outcome);
            }
            if !options.continue_on_error
                && options.max_errors > 0
                && result.counts.error_count() >= options.max_errors as u64
            {
                return Ok(result.aborted("max errors exceeded"));
            }
            if options.is_cancelled() {
                return Ok(result.aborted(CANCELLED_ABORT_REASON));
            }
        }
    }
    if !batch.is_empty() {
        let seqr = BatchSequencer::new();
        let ticket = seqr.ticket(0);
        let results = writer
            .write_batch_seq(&tenant, &sub, batch, options, ticket, seq)
            .await?;
        for r in results {
            result.counts.increment(r.outcome);
        }
        if CHECK_TRAILING.with(|c| c.get()) {
            if !options.continue_on_error
                && options.max_errors > 0
                && result.counts.error_count() >= options.max_errors as u64
            {
                return Ok(result.aborted("max errors exceeded"));
            }
            if options.is_cancelled() {
                return Ok(result.aborted(CANCELLED_ABORT_REASON));
            }
        }
    }
    Ok(result)
}

thread_local! {
    /// Apply the post-batch checks after the trailing partial batch too (the
    /// one deliberate difference found), to look for any other mismatch.
    static CHECK_TRAILING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[tokio::test(flavor = "current_thread")]
async fn pipeline_counts_match_modulo_trailing_check() {
    CHECK_TRAILING.with(|c| c.set(true));
    for seed in 0..1500u64 {
        let mut rng = Rng::new(seed + 1000);
        let lines = rng.below(50);
        let dup = rng.below(2) == 0;
        let file = gen_file(&mut rng, lines, true, dup);
        let batch_size = 1 + rng.below(6) as u32;
        let fail_lines: HashSet<u64> = (1..=lines).filter(|_| rng.below(5) == 0).collect();
        let resume = if rng.below(3) == 0 {
            rng.below(lines + 2)
        } else {
            0
        };
        let variant = rng.below(4);
        let base = BulkProcessingOptions::new()
            .with_batch_size(batch_size)
            .with_resume_after_line(resume);
        let options = match variant {
            0 => base,
            1 => base.with_continue_on_error(false),
            2 => base
                .with_continue_on_error(false)
                .with_max_errors(1 + rng.below(4) as u32),
            _ => base.with_max_errors(1 + rng.below(4) as u32),
        };
        let delays: Arc<Vec<u64>> = Arc::new((0..lines + 2).map(|_| rng.below(3) * 100).collect());
        let mut w = ModelWriter::new(64, Arc::new(Mutex::new(Model::default())));
        w.fail_lines = Arc::new(fail_lines.clone());
        w.delays = Arc::clone(&delays);
        let serial = drive_serial(&w, &file.text, &options).await.unwrap();
        for k in [1u32, 4] {
            let mut w = ModelWriter::new(64, Arc::new(Mutex::new(Model::default())));
            w.fail_lines = Arc::new(fail_lines.clone());
            w.delays = Arc::clone(&delays);
            let got = drive_new(w, &file.text, &options.clone().with_batch_writers(k))
                .await
                .unwrap();
            assert_eq!(
                summary(&got),
                summary(&serial),
                "seed {seed} variant {variant} k {k} bs {batch_size} resume {resume}"
            );
        }
    }
}

impl ModelWriter {
    /// Runs batch `seq`'s script with a standalone ticket (serial reference).
    async fn write_batch_seq(
        &self,
        tenant: &TenantContext,
        sub: &SubmissionId,
        entries: Vec<NdjsonEntry>,
        options: &BulkProcessingOptions,
        _ticket: BatchTicket,
        seq: u64,
    ) -> StorageResult<Vec<BulkEntryResult>> {
        // Map the serial seq onto the script by a sequencer that starts at seq.
        let seqr = BatchSequencer::new();
        seqr.state.send_modify(|s| {
            s.next_begin = seq;
            s.next_charge = seq;
        });
        let ticket = seqr.ticket(seq);
        self.write_batch(tenant, sub, "m1", entries, options, ticket)
            .await
    }
}

fn summary(result: &StreamProcessingResult) -> serde_json::Value {
    serde_json::to_value(result).unwrap()
}

/// Every data line at or under the watermark has been charged, nothing was
/// counted twice, nothing skipped, and the charged seqs are a contiguous prefix.
fn check_resume_invariants(m: &Model, file: &File, label: &str) {
    assert!(
        m.double_counted.is_empty(),
        "{label}: double-counted {:?}",
        m.double_counted
    );
    assert!(
        m.skipped.is_empty(),
        "{label}: skipped uncommitted lines {:?}",
        m.skipped
    );
    let mut seqs = m.charged_seqs.clone();
    let sorted = {
        let mut s = seqs.clone();
        s.sort();
        s
    };
    assert_eq!(seqs, sorted, "{label}: charged out of order");
    seqs.dedup();
    for (i, s) in seqs.iter().enumerate() {
        assert_eq!(
            *s, i as u64,
            "{label}: charged seqs not a contiguous prefix: {seqs:?}"
        );
    }
    for line in file.data_lines.range(..=m.watermark) {
        assert!(
            m.charged_lines.contains(line),
            "{label}: line {line} is under the watermark {} but was never charged",
            m.watermark
        );
    }
}

/// Random faults, random crash (future dropped), then a clean resume from the
/// watermark: no line is ever skipped or double-counted, and the resume
/// charges every data line.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pipeline_resume_after_any_fault_or_crash_never_skips_a_line() {
    let faults = [
        None,
        Some(Fault::ErrBeforeTurn),
        Some(Fault::PanicBeforeTurn),
        Some(Fault::PanicInsideTurn),
        Some(Fault::ErrAfterCharge),
        Some(Fault::PanicAfterCharge),
    ];
    for seed in 0..400u64 {
        let mut rng = Rng::new(seed + 1);
        let lines = 1 + rng.below(60);
        let (rej, dup) = (rng.below(2) == 0, rng.below(3) == 0);
        let file = gen_file(&mut rng, lines, rej, dup);
        let batch_size = 1 + rng.below(5) as u32;
        let k = [1u32, 2, 4, 8][rng.below(4) as usize];
        let batches = lines / batch_size as u64 + 2;
        let delays: Vec<u64> = (0..batches).map(|_| rng.below(3) * 300).collect();
        let fault =
            faults[rng.below(faults.len() as u64) as usize].map(|f| (rng.below(batches), f));
        let crash_after = (rng.below(3) == 0).then(|| rng.below(2000));
        let model = Arc::new(Mutex::new(Model::default()));
        let mut writer = ModelWriter::new(64, Arc::clone(&model));
        writer.delays = Arc::new(delays);
        writer.fault = fault;
        let options = BulkProcessingOptions::new()
            .with_batch_size(batch_size)
            .with_batch_writers(k);
        let text = file.text.clone();
        let w = writer.clone();
        let o = options.clone();
        let handle = tokio::spawn(async move { drive_new(w, &text, &o).await });
        let outcome = match crash_after {
            Some(us) => {
                tokio::time::sleep(Duration::from_micros(us)).await;
                handle.abort();
                handle.await
            }
            None => handle.await,
        };
        // Let aborted writer tasks unwind.
        tokio::time::sleep(Duration::from_millis(5)).await;
        let label = format!(
            "seed {seed} k {k} bs {batch_size} lines {lines} fault {fault:?} crash {crash_after:?} outcome {:?}",
            outcome
                .as_ref()
                .map(|r| r
                    .as_ref()
                    .map(|r| r.lines_processed)
                    .map_err(|e| e.to_string()))
                .map_err(|e| e.is_panic())
        );
        {
            let m = model.lock().unwrap();
            check_resume_invariants(&m, &file, &label);
        }
        // Resume from the watermark, no faults.
        let watermark = model.lock().unwrap().watermark;
        model.lock().unwrap().charged_seqs.clear();
        let mut resume = ModelWriter::new(64, Arc::clone(&model));
        resume.delays = Arc::new((0..batches).map(|_| rng.below(3) * 200).collect());
        let options = options.with_resume_after_line(watermark);
        let result = drive_new(resume, &file.text, &options).await;
        assert!(result.is_ok(), "{label}: resume failed {:?}", result.err());
        let m = model.lock().unwrap();
        check_resume_invariants(&m, &file, &format!("{label} (resume)"));
        let missing: Vec<u64> = file
            .data_lines
            .difference(&m.charged_lines)
            .copied()
            .collect();
        assert!(
            missing.is_empty(),
            "{label}: after resume, never charged: {missing:?}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn pipeline_single_writer_cancellation_matches_modulo_trailing_check() {
    CHECK_TRAILING.with(|c| c.set(true));
    single_writer_cancel_body().await
}

async fn single_writer_cancel_body() {
    for seed in 0..200u64 {
        let mut rng = Rng::new(seed + 5000);
        let lines = 1 + rng.below(40);
        let file = gen_file(&mut rng, lines, true, false);
        let batch_size = 1 + rng.below(6) as u32;
        let batches = lines / batch_size as u64 + 1;
        let cancel_seq = rng.below(batches);
        let run = |serial: bool| {
            let token = CancelToken::new();
            let mut w = ModelWriter::new(64, Arc::new(Mutex::new(Model::default())));
            w.cancel_at = Some((cancel_seq, token.clone()));
            let options = BulkProcessingOptions::new()
                .with_batch_size(batch_size)
                .with_batch_writers(1)
                .with_cancel(token);
            let text = file.text.clone();
            async move {
                if serial {
                    drive_serial(&w, &text, &options).await.unwrap()
                } else {
                    drive_new(w, &text, &options).await.unwrap()
                }
            }
        };
        let serial = run(true).await;
        let got = run(false).await;
        assert_eq!(
            summary(&got),
            summary(&serial),
            "seed {seed} bs {batch_size} lines {lines} cancel at batch {cancel_seq}"
        );
    }
}

/// With K = 4, how many batches start writing after the cancel token is set
/// (the serial loop: none).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pipeline_multi_writer_cancellation_writes_after_cancel() {
    let token = CancelToken::new();
    let model = Arc::new(Mutex::new(Model::default()));
    let mut w = ModelWriter::new(64, Arc::clone(&model));
    w.cancel_at = Some((0, token.clone()));
    // Batch 0 is slow, the rest quick.
    w.delays = Arc::new(vec![20_000]);
    let file = gen_file(&mut Rng::new(7), 100, false, false);
    let options = BulkProcessingOptions::new()
        .with_batch_size(5)
        .with_batch_writers(4)
        .with_cancel(token);
    let got = drive_new(w, &file.text, &options).await.unwrap();
    let m = model.lock().unwrap();
    eprintln!(
        "K=4 cancel at batch 0: written {:?}, started after cancel {}, result lines {} aborted {}",
        m.written_seqs, m.started_after_cancel, got.lines_processed, got.aborted
    );
    assert_eq!(
        m.started_after_cancel, 0,
        "batches started writing after the cancel was set"
    );
}

/// Memory bound: the head batch stalls after its charge (e.g. a slow
/// index-sink enqueue). How far does admission run ahead of the apply log?
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pipeline_admission_is_bounded_while_the_head_batch_stalls() {
    let model = Arc::new(Mutex::new(Model::default()));
    let mut w = ModelWriter::new(64, Arc::clone(&model));
    w.stall_after_complete = Some((0, Duration::from_millis(300)));
    let file = gen_file(&mut Rng::new(9), 2000, false, false);
    let options = BulkProcessingOptions::new()
        .with_batch_size(10)
        .with_batch_writers(4);
    let m2 = Arc::clone(&model);
    let probe = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        m2.lock().unwrap().written_seqs.len()
    });
    let got = drive_new(w, &file.text, &options).await.unwrap();
    let written_while_head_stalled = probe.await.unwrap();
    eprintln!(
        "batches written while batch 0 stalled post-charge: {written_while_head_stalled} (K=4); total lines {}",
        got.lines_processed
    );
    assert!(
        written_while_head_stalled <= 2 * 4,
        "admission ran {written_while_head_stalled} batches ahead of an unapplied head batch"
    );
}

/// An `AsyncRead` that serves `limit` bytes of `data` and then panics — a bug
/// in a decrypting / decompressing / progress-counting reader layer.
struct PanickingRead {
    data: Vec<u8>,
    pos: usize,
    limit: usize,
}

impl tokio::io::AsyncRead for PanickingRead {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if self.pos >= self.limit {
            panic!("reader layer bug after {} bytes", self.pos);
        }
        let end = (self.pos + 16).min(self.limit).min(self.data.len());
        let chunk = self.data[self.pos..end].to_vec();
        buf.put_slice(&chunk);
        self.pos = end;
        std::task::Poll::Ready(Ok(()))
    }
}

/// A panic in the input reader must not read as a clean end of file: the
/// serial loop read inline, so the panic took the file down with it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_a_reader_panic_is_not_a_clean_end_of_file() {
    let file = gen_file(&mut Rng::new(11), 40, false, false);
    let data = file.text.clone().into_bytes();
    let limit = data.len() / 2;
    let reader: Box<dyn AsyncBufRead + Send + Unpin> =
        Box::new(tokio::io::BufReader::with_capacity(
            32,
            PanickingRead {
                data,
                pos: 0,
                limit,
            },
        ));
    let model = Arc::new(Mutex::new(Model::default()));
    let writer = ModelWriter::new(64, Arc::clone(&model));
    let options = BulkProcessingOptions::new()
        .with_batch_size(5)
        .with_batch_writers(4);
    let handle = tokio::spawn(async move {
        drive_ndjson_stream(
            writer,
            &tenant(),
            &SubmissionId::generate("review"),
            "m1",
            "Patient",
            reader,
            &options,
        )
        .await
    });
    let outcome = handle.await;
    let charged = model.lock().unwrap().charged_lines.len();
    match outcome {
        Ok(Ok(result)) => panic!(
            "reader panicked half-way, yet the stream returned Ok: lines_processed {} aborted {} \
             ({} of {} data lines charged) — the worker would mark the file done",
            result.lines_processed,
            result.aborted,
            charged,
            file.data_lines.len()
        ),
        Ok(Err(e)) => eprintln!("reader panic surfaced as error: {e}"),
        Err(join) => eprintln!("reader panic propagated: panic={}", join.is_panic()),
    }
}
