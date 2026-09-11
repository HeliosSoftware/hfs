//! Temp-spooled receipt construction for bulk-submit result artifacts (#982).
//!
//! When a manifest finishes, `$bulk-submit` turns its stored per-entry results
//! into `output` receipts — one NDJSON part per resource type, alphabetically
//! ordered with a dense part index — plus a single aggregated `error` part.
//! Constructing those parts used to hold every receipt line in memory: one
//! `Vec<String>` per resource type and one more for errors. A manifest with
//! millions of entries, or a few oversized OperationOutcomes, grew that to the
//! size of the whole receipt set.
//!
//! [`ReceiptSpools`] replaces the vectors with spool files in a per-run
//! temporary directory. Rows stream to disk as they arrive and are replayed
//! afterwards one part at a time, so the heap holds the entry page currently
//! being drained, one serialized row, a fixed set of buffers, and metadata that
//! grows with the observed resource types, issue severities, and artifact count
//! rather than with the number of receipts.
//!
//! Working memory in bytes is `O(Pmax + Lmax + B + M)`: the largest materialized
//! page, largest serialized row, fixed I/O workspace, and retained metadata.
//! Requesting 1000 results limits rows, not page or row bytes. This adds no
//! per-row size cap or whole-process memory bound.
//!
//! Invariants the replay path relies on:
//!
//! - Spool file names come from a counter (`000001.ndjson`), never from a
//!   resource type: types are arbitrary provider input and must not reach the
//!   filesystem.
//! - At most [`MAX_OPEN_SPOOLS`] spool files are open. The least recently
//!   written one is flushed and closed before another is opened, so a manifest
//!   with more resource types than that never widens the handle set. Replay
//!   opens one reader at a time, after every writer has been closed.
//! - The first open of a spool creates it (`create_new`); every later open
//!   appends and never creates. A spool that vanished underneath the worker
//!   fails loudly instead of replaying as an empty part.
//! - Every append writes `row` + `\n` and counts both. Nothing else counts
//!   bytes, and replay copies the stored bytes verbatim without re-serializing
//!   or parsing them.
//! - Dropping the value removes the temp directory. Open writers are declared
//!   before the directory, so they close first. An abrupt process kill can
//!   leave the directory behind.

use std::collections::{BTreeMap, VecDeque};

use tokio::io::{AsyncWriteExt, BufWriter};

use crate::error::{BackendError, StorageError, StorageResult};

/// Maximum number of spool files kept open at once, the error spool included.
pub(crate) const MAX_OPEN_SPOOLS: usize = 8;

/// Bytes of buffering per spool writer, the bound requested from the underlying
/// Tokio file for both appends and replay reads, and the size of the fixed
/// chunk replay copies. Every spool I/O buffer is bounded by this constant.
pub(crate) const SPOOL_BUFFER_BYTES: usize = 64 * 1024;

fn spool_error(message: String, source: std::io::Error) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "bulk-submit-receipts".to_string(),
        message: format!("{message}: {source}"),
        source: None,
    })
}

/// One spooled artifact: its file name plus the exact counters replay needs.
pub(crate) struct Spool {
    /// File name inside the temp directory; always a decimal counter.
    file: String,
    /// Rows appended so far. Each was written with exactly one trailing newline.
    pub(crate) lines: u64,
    /// Bytes appended so far, every row's trailing newline included.
    pub(crate) bytes: u64,
    /// Set once the file has been created. The first open must create the file;
    /// later opens must append to it rather than recreate a missing one.
    created: bool,
}

/// Which spool an append belongs to.
enum Slot<'a> {
    /// The `output` spool of one resource type.
    Output(&'a str),
    /// The aggregated `error` spool.
    Error,
}

/// An open append writer for one spool file.
struct OpenSpool {
    file: String,
    writer: BufWriter<tokio::fs::File>,
}

/// Temp-directory spools for one manifest's receipts.
pub(crate) struct ReceiptSpools {
    /// Open append writers, least recently written first.
    open: VecDeque<OpenSpool>,
    /// One spool per resource type that produced at least one success receipt.
    outputs: BTreeMap<String, Spool>,
    /// The aggregated `error` spool, absent until its first row.
    error: Option<Spool>,
    /// Counter behind the numeric spool file names.
    next_file: u64,
    /// Declared last on purpose: fields drop in declaration order, so the open
    /// writers close before the directory removal this value performs.
    dir: tempfile::TempDir,
}

impl ReceiptSpools {
    /// Creates a spool directory under the platform temp directory, which
    /// `TMPDIR` selects.
    pub(crate) fn new() -> StorageResult<Self> {
        let dir = tempfile::Builder::new()
            .prefix("hfs-bulk-submit-receipts-")
            .tempdir()
            .map_err(|e| spool_error("create receipt spool directory".to_string(), e))?;
        Ok(Self::in_dir(dir))
    }

    /// Spools inside a caller-owned temporary directory.
    pub(crate) fn in_dir(dir: tempfile::TempDir) -> Self {
        Self {
            open: VecDeque::new(),
            outputs: BTreeMap::new(),
            error: None,
            next_file: 0,
            dir,
        }
    }

    /// Streams one success receipt to its resource type's spool.
    pub(crate) async fn push_output(
        &mut self,
        resource_type: &str,
        row: &str,
    ) -> StorageResult<()> {
        if !self.outputs.contains_key(resource_type) {
            let spool = self.allocate();
            self.outputs.insert(resource_type.to_string(), spool);
        }
        self.append(Slot::Output(resource_type), row).await
    }

    /// Streams one `error` receipt to the aggregated error spool.
    pub(crate) async fn push_error(&mut self, row: &str) -> StorageResult<()> {
        if self.error.is_none() {
            let spool = self.allocate();
            self.error = Some(spool);
        }
        self.append(Slot::Error, row).await
    }

    /// Rows appended to the error spool so far. Zero means the manifest has no
    /// persisted per-entry errors.
    pub(crate) fn error_rows(&self) -> u64 {
        self.error.as_ref().map_or(0, |spool| spool.lines)
    }

    /// Flushes and closes every spool writer, in least-recently-written order.
    /// Replay must not start before this returns: it publishes the counters, and
    /// those describe what is on disk, not what a `BufWriter` still holds.
    pub(crate) async fn close_writers(&mut self) -> StorageResult<()> {
        while !self.open.is_empty() {
            self.close_least_recent().await?;
        }
        Ok(())
    }

    /// Moves the observed `output` spools out for replay, alphabetical by
    /// resource type. Types that produced no success receipt have no spool.
    pub(crate) fn take_outputs(&mut self) -> Vec<(String, Spool)> {
        std::mem::take(&mut self.outputs).into_iter().collect()
    }

    /// Moves the aggregated `error` spool out for replay, if it has rows.
    pub(crate) fn take_error(&mut self) -> Option<Spool> {
        self.error.take()
    }

    /// Opens a bounded reader over one spool file.
    pub(crate) async fn open_reader(&self, spool: &Spool) -> StorageResult<tokio::fs::File> {
        let path = self.dir.path().join(&spool.file);
        let mut file = tokio::fs::File::open(&path)
            .await
            .map_err(|e| spool_error(format!("open receipt spool {}", path.display()), e))?;
        file.set_max_buf_size(SPOOL_BUFFER_BYTES);
        Ok(file)
    }

    /// Deletes a replayed spool file, best-effort: it lowers the disk peak while
    /// the remaining parts and any output-store scratch coexist, and a failure
    /// only leaves the file for the temp directory to reclaim.
    pub(crate) async fn remove(&self, spool: &Spool) {
        let _ = tokio::fs::remove_file(self.dir.path().join(&spool.file)).await;
    }

    fn allocate(&mut self) -> Spool {
        self.next_file += 1;
        Spool {
            file: format!("{:06}.ndjson", self.next_file),
            lines: 0,
            bytes: 0,
            created: false,
        }
    }

    async fn append(&mut self, slot: Slot<'_>, row: &str) -> StorageResult<()> {
        let (file, create) = {
            let spool = self.spool(&slot);
            (spool.file.clone(), !spool.created)
        };
        let writer = self.writer_for(&file, create).await?;
        writer
            .write_all(row.as_bytes())
            .await
            .map_err(|e| spool_error(format!("append to receipt spool {file}"), e))?;
        writer
            .write_all(b"\n")
            .await
            .map_err(|e| spool_error(format!("terminate receipt spool {file} row"), e))?;
        let spool = self.spool_mut(&slot);
        spool.created = true;
        spool.lines += 1;
        spool.bytes += row.len() as u64 + 1;
        Ok(())
    }

    fn spool(&self, slot: &Slot<'_>) -> &Spool {
        match slot {
            Slot::Output(resource_type) => self
                .outputs
                .get(*resource_type)
                .expect("output spool is allocated before its first append"),
            Slot::Error => self
                .error
                .as_ref()
                .expect("error spool is allocated before its first append"),
        }
    }

    fn spool_mut(&mut self, slot: &Slot<'_>) -> &mut Spool {
        match slot {
            Slot::Output(resource_type) => self
                .outputs
                .get_mut(*resource_type)
                .expect("output spool is allocated before its first append"),
            Slot::Error => self
                .error
                .as_mut()
                .expect("error spool is allocated before its first append"),
        }
    }

    /// Returns the append writer for a spool, opening or reopening its file.
    ///
    /// The least recently written spool is flushed and closed first when
    /// [`MAX_OPEN_SPOOLS`] writers are already open, so the open set never
    /// exceeds that bound.
    async fn writer_for(
        &mut self,
        file: &str,
        create: bool,
    ) -> StorageResult<&mut BufWriter<tokio::fs::File>> {
        let position = match self.open.iter().position(|spool| spool.file == file) {
            Some(position) => position,
            None => {
                if self.open.len() >= MAX_OPEN_SPOOLS {
                    self.close_least_recent().await?;
                }
                let writer = self.open_spool_file(file, create).await?;
                self.open.push_back(OpenSpool {
                    file: file.to_string(),
                    writer,
                });
                self.open.len() - 1
            }
        };
        if position + 1 != self.open.len() {
            let spool = self
                .open
                .remove(position)
                .expect("position came from the open writer list");
            self.open.push_back(spool);
        }
        Ok(&mut self
            .open
            .back_mut()
            .expect("the writer was just opened or moved to the back")
            .writer)
    }

    /// Opens one spool file: create the first time, append-only afterwards.
    async fn open_spool_file(
        &self,
        file: &str,
        create: bool,
    ) -> StorageResult<BufWriter<tokio::fs::File>> {
        let path = self.dir.path().join(file);
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true);
        if create {
            options.create_new(true);
        } else {
            options.append(true);
        }
        let mut handle = options
            .open(&path)
            .await
            .map_err(|e| spool_error(format!("open receipt spool {}", path.display()), e))?;
        handle.set_max_buf_size(SPOOL_BUFFER_BYTES);
        Ok(BufWriter::with_capacity(SPOOL_BUFFER_BYTES, handle))
    }

    /// Flushes and closes the least recently written spool.
    async fn close_least_recent(&mut self) -> StorageResult<()> {
        let Some(mut spool) = self.open.pop_front() else {
            return Ok(());
        };
        spool
            .writer
            .flush()
            .await
            .map_err(|e| spool_error(format!("flush receipt spool {}", spool.file), e))?;
        spool
            .writer
            .shutdown()
            .await
            .map_err(|e| spool_error(format!("close receipt spool {}", spool.file), e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    /// Spools plus the directory path, so tests can look behind the abstraction.
    fn spools() -> (ReceiptSpools, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        (ReceiptSpools::in_dir(dir), path)
    }

    fn file_names(path: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// Bytes on disk across the spool directory. Sizes come from
    /// `fs::metadata` on each path, not `DirEntry::metadata`: on Windows the
    /// latter is the directory listing's copy, which NTFS may leave stale while
    /// a writer still holds the file open, so a spool with bytes on disk can
    /// read 0.
    fn spool_bytes(path: &std::path::Path) -> u64 {
        std::fs::read_dir(path)
            .map(|entries| {
                entries
                    .map(|entry| std::fs::metadata(entry.unwrap().path()).unwrap().len())
                    .sum()
            })
            .unwrap_or(0)
    }

    async fn read_spool(spools: &ReceiptSpools, spool: &Spool) -> Vec<u8> {
        let mut reader = spools.open_reader(spool).await.unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.unwrap();
        bytes
    }

    #[tokio::test]
    async fn resource_types_never_reach_the_filesystem() {
        let (mut spools, path) = spools();
        spools
            .push_output("../../etc/Patient", "{\"reference\":\"../../etc/x\"}")
            .await
            .unwrap();
        spools
            .push_output(
                "Patient/../Patient",
                "{\"reference\":\"Patient/../Patient\"}",
            )
            .await
            .unwrap();
        spools.close_writers().await.unwrap();
        assert_eq!(file_names(&path), vec!["000001.ndjson", "000002.ndjson"]);
    }

    #[tokio::test]
    async fn interleaved_types_reopen_and_stay_within_the_handle_limit() {
        let (mut spools, path) = spools();
        let types: Vec<String> = (0..20).map(|index| format!("Type{index}")).collect();
        for round in 0..3 {
            for (index, resource_type) in types.iter().enumerate() {
                spools
                    .push_output(
                        resource_type,
                        &format!("{{\"reference\":\"{resource_type}/row-{round}\"}}"),
                    )
                    .await
                    .unwrap();
                if index % 4 == 0 {
                    spools
                        .push_error(&format!("{{\"issue\":{round}}}"))
                        .await
                        .unwrap();
                }
                assert!(
                    spools.open.len() <= MAX_OPEN_SPOOLS,
                    "more spool writers were left open than the limit allows"
                );
            }
        }

        // Twenty types plus the aggregated error spool are already in play, and
        // only the limit stays open: the cache evicts instead of widening.
        assert_eq!(spools.open.len(), MAX_OPEN_SPOOLS);
        spools.close_writers().await.unwrap();
        assert!(spools.open.is_empty());
        assert_eq!(spools.error_rows(), 15);
        assert_eq!(file_names(&path).len(), types.len() + 1);

        // More types than the handle limit, every one of them reopened across
        // rounds and complete: the bound cannot have dropped a row.
        for (resource_type, spool) in spools.take_outputs() {
            assert_eq!(spool.lines, 3);
            let rows = std::str::from_utf8(&read_spool(&spools, &spool).await)
                .unwrap()
                .to_string();
            assert_eq!(rows.lines().count(), 3, "{resource_type} rows");
            assert!(rows.contains(&format!("{resource_type}/row-2")), "{rows}");
        }
        let error = spools.take_error().unwrap();
        let rows = std::str::from_utf8(&read_spool(&spools, &error).await)
            .unwrap()
            .to_string();
        assert_eq!(rows.lines().count(), 15, "every interleaved error row");
    }

    #[tokio::test]
    async fn the_error_spool_shares_the_handle_budget() {
        let (mut spools, _path) = spools();
        spools.push_error("{\"issue\":1}").await.unwrap();
        let error_file = spools.error.as_ref().unwrap().file.clone();
        assert!(spools.open.iter().any(|open| open.file == error_file));

        for index in 0..MAX_OPEN_SPOOLS {
            spools
                .push_output(&format!("Type{index}"), "{\"reference\":\"row\"}")
                .await
                .unwrap();
            assert!(spools.open.len() <= MAX_OPEN_SPOOLS);
        }
        assert!(
            !spools.open.iter().any(|open| open.file == error_file),
            "a touched-then-idle error spool is evicted like any other"
        );
        spools.close_writers().await.unwrap();

        // The evicted spool reopens and appends on its next row rather than
        // losing what it already held.
        spools.push_error("{\"issue\":2}").await.unwrap();
        spools.close_writers().await.unwrap();
        let error = spools.take_error().unwrap();
        assert_eq!(error.lines, 2);
        assert_eq!(
            std::str::from_utf8(&read_spool(&spools, &error).await).unwrap(),
            "{\"issue\":1}\n{\"issue\":2}\n"
        );
    }

    #[tokio::test]
    async fn spool_io_buffers_are_capped() {
        let (mut spools, path) = spools();
        spools
            .push_output("Patient", "{\"reference\":\"Patient/caps\"}")
            .await
            .unwrap();
        spools
            .push_error("{\"resourceType\":\"OperationOutcome\"}")
            .await
            .unwrap();
        assert_eq!(spools.open.len(), 2);
        for open in &spools.open {
            assert_eq!(open.writer.get_ref().max_buf_size(), SPOOL_BUFFER_BYTES);
        }
        // The append path buffers rows instead of flushing each one, and a row
        // at least a buffer long cannot be buffered at all: it goes through to
        // the file. (`BufWriter` exposes the buffer, not its capacity.)
        assert_eq!(spool_bytes(&path), 0);
        assert!(
            spools
                .open
                .iter()
                .any(|open| !open.writer.buffer().is_empty()),
            "rows are buffered, not flushed per row"
        );
        spools
            .push_output("Patient", &"x".repeat(SPOOL_BUFFER_BYTES * 2 + 17))
            .await
            .unwrap();
        assert!(spool_bytes(&path) >= SPOOL_BUFFER_BYTES as u64);
        spools.close_writers().await.unwrap();

        let (_, spool) = spools.take_outputs().pop().unwrap();
        let reader = spools.open_reader(&spool).await.unwrap();
        assert_eq!(reader.max_buf_size(), SPOOL_BUFFER_BYTES);
    }

    #[tokio::test]
    async fn a_spool_name_collision_fails_the_first_open() {
        let (mut spools, path) = spools();
        let collided = path.join("000001.ndjson");
        tokio::fs::write(&collided, b"pre-existing bytes")
            .await
            .unwrap();

        let error = spools.push_output("Patient", "row").await.unwrap_err();
        assert!(error.to_string().contains("open receipt spool"), "{error}");
        assert_eq!(
            tokio::fs::read(&collided).await.unwrap(),
            b"pre-existing bytes",
            "a colliding spool is never reused or truncated"
        );
        drop(spools);
        assert!(!path.exists(), "the run still cleans its directory up");
    }

    /// Swap the only open writer for one over a read-only handle. Rows smaller
    /// than the buffer are still accepted locally; reaching the file fails with
    /// `EBADF`, deterministically and without a full device. The handle copies
    /// the spool's staging bound too, so a row past it fails on the write that
    /// carries it rather than being staged whole and failing on a later flush.
    async fn attach_read_only_writer(spools: &mut ReceiptSpools) {
        let file = spools.open.back().unwrap().file.clone();
        let mut read_only = tokio::fs::File::open(spools.dir.path().join(&file))
            .await
            .unwrap();
        read_only.set_max_buf_size(SPOOL_BUFFER_BYTES);
        spools.open.back_mut().unwrap().writer =
            BufWriter::with_capacity(SPOOL_BUFFER_BYTES, read_only);
    }

    #[tokio::test]
    async fn append_failures_surface_and_clean_up() {
        let (mut spools, path) = spools();
        spools.push_output("Patient", "row").await.unwrap();
        attach_read_only_writer(&mut spools).await;

        // Past both the writer's buffer and the handle's staging bound, so no
        // layer can accept the whole row locally: the append has to fail on the
        // write that carries it.
        let error = spools
            .push_output("Patient", &"x".repeat(SPOOL_BUFFER_BYTES * 2))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("append to receipt spool"),
            "{error}"
        );
        drop(spools);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn flush_failures_surface_and_clean_up() {
        let (mut spools, path) = spools();
        spools.push_output("Patient", "row").await.unwrap();
        attach_read_only_writer(&mut spools).await;

        // The row fits the buffer, so the failure has to land on the flush.
        spools.push_output("Patient", "second row").await.unwrap();
        let error = spools.close_writers().await.unwrap_err();
        assert!(error.to_string().contains("flush receipt spool"), "{error}");
        drop(spools);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn reopening_a_vanished_spool_fails_instead_of_restarting_empty() {
        let (mut spools, path) = spools();
        spools
            .push_output("Patient", "{\"reference\":\"Patient/one\"}")
            .await
            .unwrap();
        spools.close_writers().await.unwrap();
        let file = spools.outputs["Patient"].file.clone();
        tokio::fs::remove_file(path.join(&file)).await.unwrap();

        let error = spools
            .push_output("Patient", "{\"reference\":\"Patient/two\"}")
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("open receipt spool"),
            "a vanished spool must fail loudly: {error}"
        );
        assert!(!path.join(&file).exists(), "no empty file was created");
    }

    #[tokio::test]
    async fn rows_round_trip_byte_exact_including_oversized_ones() {
        let (mut spools, path) = spools();
        let small = "{\"reference\":\"Patient/small\"}";
        let oversized = format!(
            "{{\"resourceType\":\"OperationOutcome\",\"issue\":[{{\"diagnostics\":\"{}\"}}]}}",
            "x".repeat(SPOOL_BUFFER_BYTES * 2 + 7)
        );
        let error_row = "{\"resourceType\":\"OperationOutcome\"}";
        spools.push_output("Patient", small).await.unwrap();
        spools.push_output("Patient", &oversized).await.unwrap();
        spools.push_error(error_row).await.unwrap();
        spools.close_writers().await.unwrap();

        // Counters include exactly one newline per row.
        let error = spools.take_error().unwrap();
        assert_eq!(error.lines, 1);
        assert_eq!(error.bytes, (error_row.len() + 1) as u64);
        assert_eq!(
            read_spool(&spools, &error).await,
            format!("{error_row}\n").into_bytes()
        );

        let outputs = spools.take_outputs();
        assert_eq!(outputs.len(), 1);
        let (resource_type, spool) = &outputs[0];
        assert_eq!(resource_type, "Patient");
        assert_eq!(spool.lines, 2);
        assert_eq!(spool.bytes, (small.len() + oversized.len() + 2) as u64);
        assert_eq!(
            read_spool(&spools, spool).await,
            format!("{small}\n{oversized}\n").into_bytes()
        );

        // Replay drops each spool once its part is finalized.
        spools.remove(spool).await;
        assert!(!path.join(&spool.file).exists());
    }

    #[tokio::test]
    async fn the_error_spool_appears_only_with_its_first_row() {
        let (mut spools, path) = spools();
        assert_eq!(spools.error_rows(), 0);
        assert!(spools.take_error().is_none());
        spools.push_output("Patient", "row").await.unwrap();
        assert_eq!(spools.error_rows(), 0);
        spools.push_error("boom").await.unwrap();
        assert_eq!(spools.error_rows(), 1);
        spools.close_writers().await.unwrap();

        // A success-only manifest leaves no error artifact behind.
        assert_eq!(file_names(&path), vec!["000001.ndjson", "000002.ndjson"]);
    }

    #[tokio::test]
    async fn dropping_the_spools_removes_the_directory_with_open_writers() {
        let (mut spools, path) = spools();
        spools
            .push_output("Patient", "{\"reference\":\"Patient/open\"}")
            .await
            .unwrap();
        assert!(!spools.open.is_empty());
        drop(spools);
        assert!(
            !path.exists(),
            "spool directory must not outlive the spools"
        );
    }
}
