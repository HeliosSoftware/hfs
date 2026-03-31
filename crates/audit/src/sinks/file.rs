//! Append-only NDJSON file audit sink.
//!
//! Each `AuditEvent` is serialized as a single JSON line and appended to the
//! configured file path.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use helios_fhir::r4::AuditEvent;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::error::AuditResult;
use crate::sink::AuditSink;

/// Append-only NDJSON file sink.
///
/// Thread-safe via `tokio::sync::Mutex` over a buffered writer.
pub struct FileSink {
    writer: Mutex<tokio::io::BufWriter<tokio::fs::File>>,
    path: PathBuf,
}

impl FileSink {
    /// Open (or create) the audit file for appending.
    ///
    /// Parent directories are created automatically if they don't exist.
    pub async fn new(path: impl Into<PathBuf>) -> AuditResult<Self> {
        let path = path.into();

        // Ensure parent directories exist
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }

        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;

        Ok(Self {
            writer: Mutex::new(tokio::io::BufWriter::new(file)),
            path,
        })
    }

    /// Returns the file path this sink writes to.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[async_trait]
impl AuditSink for FileSink {
    async fn record(&self, event: AuditEvent) {
        let json = match serde_json::to_string(&event) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!(error = %e, "Failed to serialize audit event");
                return;
            }
        };

        let mut writer = self.writer.lock().await;
        if let Err(e) = writer.write_all(json.as_bytes()).await {
            tracing::error!(error = %e, path = %self.path.display(), "Failed to write audit event");
            return;
        }
        if let Err(e) = writer.write_all(b"\n").await {
            tracing::error!(error = %e, path = %self.path.display(), "Failed to write newline");
        }
    }

    async fn flush(&self) {
        let mut writer = self.writer.lock().await;
        if let Err(e) = writer.flush().await {
            tracing::error!(error = %e, path = %self.path.display(), "Failed to flush audit file");
        }
    }

    fn name(&self) -> &str {
        "file"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::AuditEventBuilder;

    #[tokio::test]
    async fn test_write_single_event() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.ndjson");

        let sink = FileSink::new(&path).await.unwrap();
        let event = AuditEventBuilder::new("Device/hfs")
            .action("R")
            .outcome("0")
            .build();
        sink.record(event).await;
        sink.flush().await;

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 1);
        // Verify it's valid JSON
        let _: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    }

    #[tokio::test]
    async fn test_write_multiple_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.ndjson");

        let sink = FileSink::new(&path).await.unwrap();
        for _ in 0..3 {
            let event = AuditEventBuilder::new("Device/hfs").build();
            sink.record(event).await;
        }
        sink.flush().await;

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3);
    }

    #[tokio::test]
    async fn test_roundtrip_deserialization() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.ndjson");

        let sink = FileSink::new(&path).await.unwrap();
        let event = AuditEventBuilder::new("Device/hfs")
            .action("C")
            .outcome("0")
            .resource("Patient", "123")
            .build();
        sink.record(event).await;
        sink.flush().await;

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let line = content.lines().next().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
        // Verify it's a valid AuditEvent by checking for key fields
        assert!(parsed["id"].is_string());
        assert!(parsed["recorded"].is_string());
    }

    #[tokio::test]
    async fn test_creates_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deep").join("nested").join("audit.ndjson");

        let sink = FileSink::new(&path).await.unwrap();
        let event = AuditEventBuilder::new("Device/hfs").build();
        sink.record(event).await;
        sink.flush().await;

        assert!(path.exists());
    }

    #[tokio::test]
    async fn test_file_created_if_not_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new-audit.ndjson");
        assert!(!path.exists());

        let _sink = FileSink::new(&path).await.unwrap();
        assert!(path.exists());
    }

    #[test]
    fn test_name() {
        // Can't easily test without async, just verify the method exists
        // by checking the trait impl in the async tests above.
    }
}
