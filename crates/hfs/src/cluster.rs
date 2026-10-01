//! Cluster-mode configuration fail-fast validation.
//!
//! `HFS_CLUSTER=true` declares this process one of N instances behind a load
//! balancer. Several configurations that are fine single-instance silently
//! lose data when clustered — a single-writer SQLite file as the primary,
//! bulk output on node-local disk, a node-local audit file, a per-process
//! bulk-export job store, an in-process async-job store. This module refuses
//! to boot on them instead, and warns about the ones that are merely
//! per-instance today.
//!
//! The checks live here — not in `helios-rest`'s `ServerConfig::validate()` —
//! because they span config domains that only the `hfs` binary sees
//! together: the rest server config and the audit config (`HFS_AUDIT_*`). The
//! validator is a pure function over a [`ClusterConfigView`], so the whole
//! refusal table is unit-testable without environment-variable mutation
//! (which races under parallel `cargo test`).

use helios_audit::AuditBackend;
use helios_persistence::BackendKind;

/// The cluster-relevant slice of the assembled boot configuration.
///
/// Assembled in `main()` from `ServerConfig` and `AuditConfig` after each has
/// been parsed from the environment.
pub struct ClusterConfigView<'a> {
    /// `HFS_CLUSTER`.
    pub cluster: bool,
    /// Primary storage backend kind (from `HFS_STORAGE_BACKEND`).
    pub primary_backend: BackendKind,
    /// Raw `HFS_JOB_STORE_BACKEND` value ("" when unset).
    pub job_store_backend: &'a str,
    /// `HFS_BULK_EXPORT_ENABLED`.
    pub bulk_export_enabled: bool,
    /// `HFS_BULK_EXPORT_OUTPUT_BACKEND`.
    pub bulk_export_output_backend: &'a str,
    /// `HFS_BULK_SUBMIT_ENABLED`.
    pub bulk_submit_enabled: bool,
    /// `HFS_BULK_SUBMIT_OUTPUT_BACKEND`.
    pub bulk_submit_output_backend: &'a str,
    /// `HFS_AUDIT_BACKEND` (`None` means audit is disabled).
    pub audit_backend: AuditBackend,
    /// `HFS_SOF_ENABLED`.
    pub sof_enabled: bool,
    /// `HFS_EXPORT_SINK` (the SQL-on-FHIR `$sql-export` output sink).
    pub export_sink: &'a str,
}

/// What the validator concluded: `errors` refuse the boot, `warnings` are
/// logged and the boot proceeds.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ClusterVerdict {
    /// Configurations that cannot run as one of N instances.
    pub errors: Vec<String>,
    /// Configurations that run, but with a per-instance caveat the operator
    /// should know about.
    pub warnings: Vec<String>,
}

/// Checks a configuration against the cluster refusal table.
///
/// Every check is a no-op when `cluster` is false. When clustered, all
/// violations are collected (not just the first) so an operator can fix a
/// deployment in one pass. Each message names the offending environment
/// variable and the safe alternative.
pub fn validate_cluster_config(view: &ClusterConfigView<'_>) -> ClusterVerdict {
    let mut verdict = ClusterVerdict::default();
    if !view.cluster {
        return verdict;
    }

    // F1 — the primary backend must be shared across instances.
    if view.primary_backend == BackendKind::Sqlite {
        verdict.errors.push(
            "HFS_CLUSTER=true requires a shared primary backend, but HFS_STORAGE_BACKEND \
             resolves to sqlite — a single-writer local file that cannot be shared between \
             instances. Use postgres, mongodb, s3, or an *-elasticsearch mode over them."
                .to_string(),
        );
    }

    // F2 — bulk output written to node-local disk 404s when the download
    // request lands on a different instance than the writer.
    if view.bulk_export_enabled && view.bulk_export_output_backend == "local-fs" {
        verdict.errors.push(
            "HFS_CLUSTER=true requires shared bulk-export output, but \
             HFS_BULK_EXPORT_OUTPUT_BACKEND=local-fs writes to node-local disk, so downloads \
             routed to another instance return 404. Use s3 (or set HFS_BULK_EXPORT_ENABLED=false)."
                .to_string(),
        );
    }
    if view.bulk_submit_enabled && view.bulk_submit_output_backend == "local-fs" {
        verdict.errors.push(
            "HFS_CLUSTER=true requires shared bulk-submit output, but \
             HFS_BULK_SUBMIT_OUTPUT_BACKEND=local-fs writes to node-local disk, so artifacts \
             are invisible to other instances. Use s3 (or set HFS_BULK_SUBMIT_ENABLED=false)."
                .to_string(),
        );
    }

    // F3 — under a MongoDB or S3 primary the bulk-export job store is a
    // node-local SQLite sidecar (worst case a per-process temp file), so job
    // state is invisible to the other instances: a poll routed elsewhere
    // 404s and a redeploy loses the queue. Bulk *submit* has native job
    // stores on both, so only export is affected.
    if view.bulk_export_enabled
        && matches!(view.primary_backend, BackendKind::MongoDB | BackendKind::S3)
    {
        verdict.errors.push(format!(
            "HFS_CLUSTER=true cannot run bulk export on a {} primary: its job store is a \
             node-local SQLite sidecar, so job state is invisible to other instances. Use a \
             postgres primary for bulk export, or set HFS_BULK_EXPORT_ENABLED=false.",
            view.primary_backend
        ));
    }

    // F4 — a node-local audit file yields a fragmented, restart-lossy trail.
    if view.audit_backend == AuditBackend::File {
        verdict.errors.push(
            "HFS_CLUSTER=true requires a shared audit sink, but HFS_AUDIT_BACKEND=file \
             writes NDJSON to node-local (often ephemeral) disk, fragmenting the audit \
             trail across instances. Use database or cloudwatch."
                .to_string(),
        );
    }

    // An explicitly requested in-process job store contradicts the cluster
    // declaration (unset resolves to `database` under HFS_CLUSTER).
    if view.job_store_backend.trim().eq_ignore_ascii_case("memory") {
        verdict.errors.push(
            "HFS_CLUSTER=true is incompatible with HFS_JOB_STORE_BACKEND=memory: in-process \
             job state is invisible to other instances and lost on restart. Remove the \
             variable (it defaults to database when clustered) or set database."
                .to_string(),
        );
    }

    // A1 — SQL-on-FHIR async export job state is still held in-process by
    // every instance, so a poll, cancel or download routed to another
    // instance 404s. This is a warning rather than a refusal because it is
    // the current state of every deployment and there is no safe value to
    // switch to yet; the operator can pin export clients to one instance.
    if view.sof_enabled {
        verdict.warnings.push(
            "SQL-on-FHIR $sql-export job state is per-instance (HFS_EXPORT_CONTROLLER \
             supports only memory): poll, cancel and download requests must reach the \
             instance that accepted the export. Pin export clients to one instance or use \
             sticky sessions, or set HFS_SOF_ENABLED=false."
                .to_string(),
        );
        // A filesystem export sink is fine when the directory is shared
        // (NFS) and node-local otherwise; only the operator knows which.
        if view.export_sink.trim().eq_ignore_ascii_case("fs") {
            verdict.warnings.push(
                "HFS_EXPORT_SINK=fs stores $sql-export output under HFS_EXPORT_DIR; in a \
                 cluster that directory must be shared by every instance (e.g. NFS) or \
                 downloads routed to another instance return 404. Use s3 for shared, \
                 presigned output."
                    .to_string(),
            );
        }
    }

    verdict
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A view that passes every cluster check; cases below perturb one
    /// dimension at a time.
    fn safe_cluster_view() -> ClusterConfigView<'static> {
        ClusterConfigView {
            cluster: true,
            primary_backend: BackendKind::Postgres,
            job_store_backend: "",
            bulk_export_enabled: true,
            bulk_export_output_backend: "s3",
            bulk_submit_enabled: true,
            bulk_submit_output_backend: "s3",
            audit_backend: AuditBackend::Database,
            sof_enabled: true,
            export_sink: "s3",
        }
    }

    #[test]
    fn cluster_off_accepts_every_single_instance_default() {
        // The zero-config single-instance setup: sqlite, local-fs outputs,
        // file audit, fs export sink — all fine when not clustered, and
        // nothing to warn about either.
        let view = ClusterConfigView {
            cluster: false,
            primary_backend: BackendKind::Sqlite,
            job_store_backend: "memory",
            bulk_export_enabled: true,
            bulk_export_output_backend: "local-fs",
            bulk_submit_enabled: true,
            bulk_submit_output_backend: "local-fs",
            audit_backend: AuditBackend::File,
            sof_enabled: true,
            export_sink: "fs",
        };
        assert_eq!(validate_cluster_config(&view), ClusterVerdict::default());
    }

    #[test]
    fn cluster_on_accepts_a_fully_shared_configuration() {
        let verdict = validate_cluster_config(&safe_cluster_view());
        assert!(verdict.errors.is_empty(), "{:?}", verdict.errors);
        // SQL-on-FHIR async export job state is still per-instance, so even a
        // fully shared configuration carries that one caveat.
        assert_eq!(verdict.warnings.len(), 1, "{:?}", verdict.warnings);
        assert!(verdict.warnings[0].contains("$sql-export"));
    }

    /// The refusal table: one unsafe dimension at a time, each error naming
    /// its environment variable.
    #[test]
    fn cluster_on_refuses_each_unsafe_dimension() {
        struct Case {
            name: &'static str,
            view: ClusterConfigView<'static>,
            expect_var: &'static str,
        }

        let cases = vec![
            Case {
                name: "F1 sqlite primary",
                view: ClusterConfigView {
                    primary_backend: BackendKind::Sqlite,
                    ..safe_cluster_view()
                },
                expect_var: "HFS_STORAGE_BACKEND",
            },
            Case {
                name: "F2 local-fs export output",
                view: ClusterConfigView {
                    bulk_export_output_backend: "local-fs",
                    ..safe_cluster_view()
                },
                expect_var: "HFS_BULK_EXPORT_OUTPUT_BACKEND",
            },
            Case {
                name: "F2 local-fs submit output",
                view: ClusterConfigView {
                    bulk_submit_output_backend: "local-fs",
                    ..safe_cluster_view()
                },
                expect_var: "HFS_BULK_SUBMIT_OUTPUT_BACKEND",
            },
            Case {
                name: "F3 sidecar bulk-export job store under mongodb",
                view: ClusterConfigView {
                    primary_backend: BackendKind::MongoDB,
                    ..safe_cluster_view()
                },
                expect_var: "HFS_BULK_EXPORT_ENABLED",
            },
            Case {
                name: "F3 sidecar bulk-export job store under s3",
                view: ClusterConfigView {
                    primary_backend: BackendKind::S3,
                    ..safe_cluster_view()
                },
                expect_var: "HFS_BULK_EXPORT_ENABLED",
            },
            Case {
                name: "F4 file audit",
                view: ClusterConfigView {
                    audit_backend: AuditBackend::File,
                    ..safe_cluster_view()
                },
                expect_var: "HFS_AUDIT_BACKEND",
            },
            Case {
                name: "explicit memory job store",
                view: ClusterConfigView {
                    job_store_backend: "memory",
                    ..safe_cluster_view()
                },
                expect_var: "HFS_JOB_STORE_BACKEND",
            },
        ];

        for case in cases {
            let verdict = validate_cluster_config(&case.view);
            assert_eq!(
                verdict.errors.len(),
                1,
                "{}: expected exactly one error, got {:?}",
                case.name,
                verdict.errors
            );
            assert!(
                verdict.errors[0].contains(case.expect_var),
                "{}: error must name {}, got: {}",
                case.name,
                case.expect_var,
                verdict.errors[0]
            );
        }
    }

    #[test]
    fn cluster_on_collects_all_violations_at_once() {
        // A worst-case config: every refusal fires in a single boot attempt
        // (F1, F2 export, F2 submit, F4, explicit memory job store — F3 needs
        // a mongodb/s3 primary and is exclusive with F1).
        let view = ClusterConfigView {
            cluster: true,
            primary_backend: BackendKind::Sqlite,
            job_store_backend: "memory",
            bulk_export_enabled: true,
            bulk_export_output_backend: "local-fs",
            bulk_submit_enabled: true,
            bulk_submit_output_backend: "local-fs",
            audit_backend: AuditBackend::File,
            sof_enabled: true,
            export_sink: "fs",
        };
        let verdict = validate_cluster_config(&view);
        assert_eq!(verdict.errors.len(), 5, "{:?}", verdict.errors);
    }

    #[test]
    fn disabled_subsystems_are_exempt() {
        // A disabled subsystem's unsafe backend cannot hurt a cluster: bulk
        // export off + local-fs + mongodb primary (no sidecar is built), bulk
        // submit off + local-fs, audit off (None), SoF off + fs sink.
        let view = ClusterConfigView {
            primary_backend: BackendKind::MongoDB,
            bulk_export_enabled: false,
            bulk_export_output_backend: "local-fs",
            bulk_submit_enabled: false,
            bulk_submit_output_backend: "local-fs",
            audit_backend: AuditBackend::None,
            sof_enabled: false,
            export_sink: "fs",
            ..safe_cluster_view()
        };
        assert_eq!(validate_cluster_config(&view), ClusterVerdict::default());
    }

    #[test]
    fn fs_export_sink_warns_not_refuses() {
        // A shared (NFS) export directory is a legitimate deployment, so the
        // filesystem sink is a warning, not a refusal — alongside the
        // standing per-instance caveat for $sql-export job state.
        let view = ClusterConfigView {
            export_sink: "fs",
            ..safe_cluster_view()
        };
        let verdict = validate_cluster_config(&view);
        assert!(verdict.errors.is_empty(), "{:?}", verdict.errors);
        assert_eq!(verdict.warnings.len(), 2, "{:?}", verdict.warnings);
        assert!(
            verdict
                .warnings
                .iter()
                .any(|w| w.contains("HFS_EXPORT_SINK")),
            "{:?}",
            verdict.warnings
        );
    }
}
