//! Cluster-mode configuration fail-fast validation
//! (docs/cluster-capable-state-design.md §6).
//!
//! `HFS_CLUSTER=true` declares this process one of N instances behind a load
//! balancer. Several configurations that are fine single-instance silently
//! lose data or regress security when clustered (Class C1/F1/F2/F4 in the
//! design); this module refuses to boot on them instead.
//!
//! The checks live here — not in `helios-rest`'s `ServerConfig::validate()` —
//! because they span three config domains that only the `hfs` binary sees
//! together: the rest server config, the auth config (`HFS_AUTH_*`), and the
//! audit config (`HFS_AUDIT_*`). The validator is a pure function over a
//! [`ClusterConfigView`] so the whole refusal table is unit-testable without
//! environment-variable mutation (which races under parallel `cargo test`).

use helios_audit::AuditBackend;
use helios_persistence::BackendKind;

/// The cluster-relevant slice of the assembled boot configuration.
///
/// Assembled in `main()` from `ServerConfig`, `AuthConfig`, and
/// `AuditConfig` after each has been parsed from the environment.
pub struct ClusterConfigView<'a> {
    /// `HFS_CLUSTER`.
    pub cluster: bool,
    /// Primary storage backend kind (from `HFS_STORAGE_BACKEND`).
    pub primary_backend: BackendKind,
    /// Raw `HFS_JOB_STORE_BACKEND` value ("" when unset).
    pub job_store_backend: &'a str,
    /// `HFS_AUTH_ENABLED`.
    pub auth_enabled: bool,
    /// `HFS_AUTH_JTI_BACKEND`.
    pub jti_backend: &'a str,
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
}

/// Refuses configurations that cannot run as one of N instances.
///
/// Every check is a no-op when `cluster` is false; when clustered, all
/// violations are collected (not just the first) so an operator can fix a
/// deployment in one pass. Each message names the offending environment
/// variable and the safe alternative.
pub fn validate_cluster_config(view: &ClusterConfigView<'_>) -> Result<(), Vec<String>> {
    if !view.cluster {
        return Ok(());
    }

    let mut errors = Vec::new();

    // F1 — the primary backend must be shared across instances.
    if view.primary_backend == BackendKind::Sqlite {
        errors.push(
            "HFS_CLUSTER=true requires a shared primary backend, but HFS_STORAGE_BACKEND \
             resolves to sqlite — a single-writer local file that cannot be shared between \
             instances. Use postgres, mongodb, s3, or an *-elasticsearch mode over them."
                .to_string(),
        );
    }

    // C1 — a per-instance JWT replay cache is a security regression: a
    // one-time token accepted on this instance would be honored again by
    // every other instance.
    if view.auth_enabled && view.jti_backend == "memory" {
        errors.push(
            "HFS_CLUSTER=true requires a shared JWT replay cache, but \
             HFS_AUTH_JTI_BACKEND=memory is per-instance, so a one-time token replayed \
             against another instance would be accepted. Use redis (HFS_AUTH_REDIS_URL)."
                .to_string(),
        );
    }

    // F2 — bulk output written to node-local disk 404s when the download
    // request lands on a different instance than the writer.
    if view.bulk_export_enabled && view.bulk_export_output_backend == "local-fs" {
        errors.push(
            "HFS_CLUSTER=true requires shared bulk-export output, but \
             HFS_BULK_EXPORT_OUTPUT_BACKEND=local-fs writes to node-local disk, so downloads \
             routed to another instance return 404. Use s3 (or set HFS_BULK_EXPORT_ENABLED=false)."
                .to_string(),
        );
    }
    if view.bulk_submit_enabled && view.bulk_submit_output_backend == "local-fs" {
        errors.push(
            "HFS_CLUSTER=true requires shared bulk-submit output, but \
             HFS_BULK_SUBMIT_OUTPUT_BACKEND=local-fs writes to node-local disk, so artifacts \
             are invisible to other instances. Use s3 (or set HFS_BULK_SUBMIT_ENABLED=false)."
                .to_string(),
        );
    }

    // F4 — a node-local audit file yields a fragmented, restart-lossy trail.
    if view.audit_backend == AuditBackend::File {
        errors.push(
            "HFS_CLUSTER=true requires a shared audit sink, but HFS_AUDIT_BACKEND=file \
             writes NDJSON to node-local (often ephemeral) disk, fragmenting the audit \
             trail across instances. Use database or cloudwatch."
                .to_string(),
        );
    }

    // An explicitly requested in-process job store contradicts the cluster
    // declaration (unset resolves to `database` under HFS_CLUSTER).
    if view.job_store_backend.eq_ignore_ascii_case("memory") {
        errors.push(
            "HFS_CLUSTER=true is incompatible with HFS_JOB_STORE_BACKEND=memory: in-process \
             job state is invisible to other instances and lost on restart. Remove the \
             variable (it defaults to database when clustered) or set database."
                .to_string(),
        );
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
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
            auth_enabled: true,
            jti_backend: "redis",
            bulk_export_enabled: true,
            bulk_export_output_backend: "s3",
            bulk_submit_enabled: true,
            bulk_submit_output_backend: "s3",
            audit_backend: AuditBackend::Database,
        }
    }

    #[test]
    fn cluster_off_accepts_every_single_instance_default() {
        // The zero-config single-instance setup: sqlite, memory jti,
        // local-fs outputs, file audit — all fine when not clustered.
        let view = ClusterConfigView {
            cluster: false,
            primary_backend: BackendKind::Sqlite,
            job_store_backend: "memory",
            auth_enabled: true,
            jti_backend: "memory",
            bulk_export_enabled: true,
            bulk_export_output_backend: "local-fs",
            bulk_submit_enabled: true,
            bulk_submit_output_backend: "local-fs",
            audit_backend: AuditBackend::File,
        };
        assert_eq!(validate_cluster_config(&view), Ok(()));
    }

    #[test]
    fn cluster_on_accepts_a_fully_shared_configuration() {
        assert_eq!(validate_cluster_config(&safe_cluster_view()), Ok(()));
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
                name: "C1 memory jti",
                view: ClusterConfigView {
                    jti_backend: "memory",
                    ..safe_cluster_view()
                },
                expect_var: "HFS_AUTH_JTI_BACKEND",
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
            let errors = validate_cluster_config(&case.view)
                .expect_err(&format!("{} must be refused", case.name));
            assert_eq!(errors.len(), 1, "{}: expected one error", case.name);
            assert!(
                errors[0].contains(case.expect_var),
                "{}: error must name {}, got: {}",
                case.name,
                case.expect_var,
                errors[0]
            );
        }
    }

    #[test]
    fn cluster_on_collects_all_violations_at_once() {
        // A worst-case config: every check fires in a single boot attempt.
        let view = ClusterConfigView {
            cluster: true,
            primary_backend: BackendKind::Sqlite,
            job_store_backend: "memory",
            auth_enabled: true,
            jti_backend: "memory",
            bulk_export_enabled: true,
            bulk_export_output_backend: "local-fs",
            bulk_submit_enabled: true,
            bulk_submit_output_backend: "local-fs",
            audit_backend: AuditBackend::File,
        };
        let errors = validate_cluster_config(&view).unwrap_err();
        assert_eq!(errors.len(), 6);
    }

    #[test]
    fn disabled_subsystems_are_exempt() {
        // A disabled subsystem's unsafe backend cannot hurt a cluster:
        // auth off + memory jti, bulk off + local-fs, audit off (None).
        let view = ClusterConfigView {
            auth_enabled: false,
            jti_backend: "memory",
            bulk_export_enabled: false,
            bulk_export_output_backend: "local-fs",
            bulk_submit_enabled: false,
            bulk_submit_output_backend: "local-fs",
            audit_backend: AuditBackend::None,
            ..safe_cluster_view()
        };
        assert_eq!(validate_cluster_config(&view), Ok(()));
    }

    #[test]
    fn disabled_jti_boots_with_replay_protection_off_everywhere() {
        // `disabled` is uniform across instances (no false sense of shared
        // protection), so it boots; the operator doc carries the caveat.
        let view = ClusterConfigView {
            jti_backend: "disabled",
            ..safe_cluster_view()
        };
        assert_eq!(validate_cluster_config(&view), Ok(()));
    }
}
