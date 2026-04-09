// This module previously contained the `AuditEventSink` trait.
// Audit event construction and emission now lives in the `helios-audit` crate.
// Auth middleware in `helios-rest` uses `AuditSink` and `AuditEventBuilder` directly.
