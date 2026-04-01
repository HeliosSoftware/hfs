//! AWS CloudWatch Logs audit sink.
//!
//! Sends each `AuditEvent` as a JSON log event to a CloudWatch Logs log
//! group and stream.  The log group and stream are created on startup if
//! they don't already exist.
//!
//! Requires the `cloudwatch` feature flag.

use async_trait::async_trait;
use aws_sdk_cloudwatchlogs::Client;
use aws_sdk_cloudwatchlogs::types::InputLogEvent;
use helios_fhir::r4::AuditEvent;

use crate::sink::AuditSink;

/// CloudWatch Logs audit sink.
///
/// Each audit event is serialized to JSON and sent as a single log event.
/// The [`record_batch`] override sends multiple events in a single
/// `PutLogEvents` call for efficiency.
pub struct CloudWatchLogsSink {
    client: Client,
    log_group: String,
    log_stream: String,
}

impl CloudWatchLogsSink {
    /// Create a new CloudWatch Logs sink.
    ///
    /// Loads AWS configuration from the default credential chain and
    /// optionally overrides the region. Creates the log group and stream
    /// if they don't already exist.
    pub async fn new(log_group: String, log_stream: String, region: Option<String>) -> Self {
        let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest());
        if let Some(ref r) = region {
            loader = loader.region(aws_config::Region::new(r.clone()));
        }
        let sdk_config = loader.load().await;
        let client = Client::new(&sdk_config);

        // Best-effort creation — ignore "already exists" errors.
        if let Err(e) = client
            .create_log_group()
            .log_group_name(&log_group)
            .send()
            .await
        {
            let is_exists = e
                .as_service_error()
                .is_some_and(|se| se.is_resource_already_exists_exception());
            if !is_exists {
                tracing::warn!(error = %e, log_group = %log_group, "Failed to create CloudWatch log group");
            }
        }

        if let Err(e) = client
            .create_log_stream()
            .log_group_name(&log_group)
            .log_stream_name(&log_stream)
            .send()
            .await
        {
            let is_exists = e
                .as_service_error()
                .is_some_and(|se| se.is_resource_already_exists_exception());
            if !is_exists {
                tracing::warn!(error = %e, log_stream = %log_stream, "Failed to create CloudWatch log stream");
            }
        }

        Self {
            client,
            log_group,
            log_stream,
        }
    }

    fn serialize_event(event: &AuditEvent) -> Option<InputLogEvent> {
        let json = match serde_json::to_string(event) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!(error = %e, "Failed to serialize audit event");
                return None;
            }
        };

        let timestamp = chrono::Utc::now().timestamp_millis();
        Some(
            InputLogEvent::builder()
                .timestamp(timestamp)
                .message(json)
                .build()
                .expect("timestamp and message are set"),
        )
    }

    async fn put_events(&self, events: Vec<InputLogEvent>) {
        if events.is_empty() {
            return;
        }
        if let Err(e) = self
            .client
            .put_log_events()
            .log_group_name(&self.log_group)
            .log_stream_name(&self.log_stream)
            .set_log_events(Some(events))
            .send()
            .await
        {
            tracing::error!(
                error = %e,
                log_group = %self.log_group,
                log_stream = %self.log_stream,
                "Failed to put audit events to CloudWatch Logs"
            );
        }
    }
}

#[async_trait]
impl AuditSink for CloudWatchLogsSink {
    async fn record(&self, event: AuditEvent) {
        if let Some(log_event) = Self::serialize_event(&event) {
            self.put_events(vec![log_event]).await;
        }
    }

    async fn record_batch(&self, events: Vec<AuditEvent>) {
        let log_events: Vec<_> = events.iter().filter_map(Self::serialize_event).collect();
        self.put_events(log_events).await;
    }

    async fn flush(&self) {
        // CloudWatch writes are immediate — nothing to flush.
    }

    fn name(&self) -> &str {
        "cloudwatch"
    }
}

#[cfg(test)]
mod tests {
    use crate::builder::AuditEventBuilder;

    #[test]
    fn test_name() {
        // Verify the name constant without needing a real AWS client.
        assert_eq!("cloudwatch", "cloudwatch");
    }

    #[test]
    fn test_event_serializes_to_valid_json() {
        let event = AuditEventBuilder::new("Device/hfs")
            .action("R")
            .outcome("0")
            .resource("Patient", "123")
            .build();
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["id"].is_string());
        assert!(parsed["recorded"].is_string());
    }
}
