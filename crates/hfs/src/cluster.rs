//! Cluster-mode configuration fail-fast validation
//! (docs/cluster-capable-state-design.md §6).
//!
//! `HFS_CLUSTER=true` declares this process one of N instances behind a load
//! balancer. Several configurations that are fine single-instance silently
//! lose data or degrade when clustered (Class A1/F1/F2/F4 in the design);
//! this module refuses to boot on them instead. (The former C1 check — a
//! per-instance JWT replay cache — went away with the jti subsystem itself:
//! #205 made token validation stateless, so auth holds no cross-instance
//! state to guard.)
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
    /// Raw `HFS_EXPORT_CONTROLLER` value ("" when unset).
    pub export_controller: &'a str,
    /// `HFS_EXPORT_SINK`.
    pub export_sink: &'a str,
    /// Whether the Subscriptions engine is enabled (resolved
    /// `HFS_SUBSCRIPTIONS_ENABLED` AND the binary carries the feature —
    /// a subsystem that cannot run cannot hurt a cluster).
    pub subscriptions_enabled: bool,
    /// Raw `HFS_SUBSCRIPTIONS_FANOUT` value ("" when unset).
    pub subscriptions_fanout: &'a str,
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

    // A1 (#169) — SoF async export state must be cluster-shared: the
    // controller on the shared job store, and shards in shared storage.
    if view.sof_enabled {
        if view.export_controller.eq_ignore_ascii_case("memory") {
            errors.push(
                "HFS_CLUSTER=true is incompatible with HFS_EXPORT_CONTROLLER=memory: SoF \
                 export jobs would be per-instance, so poll/cancel/download on another \
                 instance 404s. Remove the variable (it defaults to database when \
                 clustered) or set database."
                    .to_string(),
            );
        }
        if view.export_sink.eq_ignore_ascii_case("fs") {
            errors.push(
                "HFS_CLUSTER=true requires shared SoF export output, but HFS_EXPORT_SINK=fs \
                 writes shards to node-local disk, so downloads routed to another instance \
                 return 404. Use s3 (or set HFS_SOF_ENABLED=false)."
                    .to_string(),
            );
        }
    }

    // B1-B5 (#170) — subscription reaction must be cluster-shared: an
    // in-process fan-out silently drops notifications for every write routed
    // to an instance that doesn't hold the registration (unlike C2's
    // warn-only per-instance JWKS, this is functional breakage).
    if view.subscriptions_enabled {
        if view.subscriptions_fanout.eq_ignore_ascii_case("memory") {
            errors.push(
                "HFS_CLUSTER=true is incompatible with HFS_SUBSCRIPTIONS_FANOUT=memory: \
                 subscription registrations, sockets, and delivery state would be \
                 per-instance, silently dropping notifications for writes served by other \
                 instances. Remove the variable (it defaults to pg-notify when clustered) \
                 or set HFS_SUBSCRIPTIONS_ENABLED=false."
                    .to_string(),
            );
        }
        if view.primary_backend != BackendKind::Postgres {
            errors.push(
                "HFS_CLUSTER=true with HFS_SUBSCRIPTIONS_ENABLED requires a postgres \
                 primary backend: the subscriptions fan-out and shared delivery state ride \
                 the primary database (LISTEN/NOTIFY), which this backend cannot provide. \
                 Use a postgres primary or set HFS_SUBSCRIPTIONS_ENABLED=false."
                    .to_string(),
            );
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// How JWKS refreshes are coordinated across instances (design §5 C2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JwksCoordinationMode {
    /// Per-instance refresh (the single-instance default).
    Local,
    /// Cluster-wide single-flight over the primary backend's shared
    /// refresh cache.
    Database,
    /// Cluster-wide single-flight over Redis (`HFS_AUTH_REDIS_URL`).
    Redis,
}

/// Resolves `HFS_AUTH_JWKS_COORDINATION` (`raw`, "" when unset).
///
/// Unlike the refusal table above, C2 is **warn-only by design**: per-instance
/// JWKS refresh is functionally correct (every node fetches the same public
/// keys) — the cost is only a thundering herd on the IdP. So under
/// `HFS_CLUSTER=true` an unset value resolves to `Database` and an explicit
/// `local` yields a warning (returned as the second tuple element), never a
/// refusal. Invalid *values* still fail fast, naming the variable.
pub fn resolve_jwks_coordination(
    raw: &str,
    cluster: bool,
    redis_url: Option<&str>,
) -> Result<(JwksCoordinationMode, Option<String>), String> {
    match raw {
        "" => {
            if cluster {
                Ok((JwksCoordinationMode::Database, None))
            } else {
                Ok((JwksCoordinationMode::Local, None))
            }
        }
        "local" => {
            let warning = cluster.then(|| {
                "HFS_CLUSTER=true with HFS_AUTH_JWKS_COORDINATION=local: every instance \
                 refreshes JWKS independently, hammering the IdP on boot and key rotation. \
                 Functionally correct, but consider database."
                    .to_string()
            });
            Ok((JwksCoordinationMode::Local, warning))
        }
        "database" => Ok((JwksCoordinationMode::Database, None)),
        "redis" => {
            if !cfg!(feature = "redis") {
                return Err(
                    "HFS_AUTH_JWKS_COORDINATION=redis requires an hfs binary built with the \
                     'redis' feature. Rebuild with --features redis, or use database."
                        .to_string(),
                );
            }
            if redis_url.is_none_or(str::is_empty) {
                return Err(
                    "HFS_AUTH_JWKS_COORDINATION=redis requires HFS_AUTH_REDIS_URL to be set."
                        .to_string(),
                );
            }
            Ok((JwksCoordinationMode::Redis, None))
        }
        other => Err(format!(
            "Unknown HFS_AUTH_JWKS_COORDINATION value '{other}'. \
             Use local, database, or redis."
        )),
    }
}

/// Warns (never refuses) when the composite secondary-backend sync outbox
/// (E1) cannot back cluster-durable delivery.
///
/// Unlike the refusal table above, this is deliberately **warn-only**:
/// `CompositeStorage` wires the durable outbox unconditionally whenever the
/// primary backend supports it (a Postgres primary) — no env var selects it,
/// so there is nothing to "refuse" here. On a non-Postgres primary,
/// composite sync falls back to the pre-existing in-memory-channel
/// behavior — the *exact same* behavior already shipped single-instance
/// today, not a cluster-introduced regression. Contrast with subscriptions'
/// hard refusal: subscription delivery is functionally *broken* (zero
/// notifications) without Postgres+pg-notify, whereas composite sync
/// degrading to best-effort in-memory is a known, tolerable, already-shipped
/// fallback. Only relevant in `Asynchronous`/`Hybrid` mode — `Synchronous`
/// blocks on the secondary write and is already durable by construction.
pub fn resolve_composite_sync_durability(
    cluster: bool,
    primary_backend: BackendKind,
    composite_secondary_present: bool,
    sync_mode_synchronous: bool,
) -> Option<String> {
    if !cluster || !composite_secondary_present || sync_mode_synchronous {
        return None;
    }
    if primary_backend != BackendKind::Postgres {
        return Some(
            "HFS_CLUSTER=true with a composite secondary backend (an *-elasticsearch storage \
             mode) on a non-postgres primary: secondary-backend sync stays on the in-memory \
             async worker (best-effort, not crash-durable) because the durable outbox seam \
             (E1) is Postgres-primary-only. This is the same fallback behavior as \
             single-instance today — no regression — but a crash or redeploy can still \
             silently lose queued secondary-index writes. Consider a postgres primary for \
             durable composite sync."
                .to_string(),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A view that passes every cluster check; cases below perturb one
    /// dimension at a time. Subscriptions are off here so the sqlite-primary
    /// case fires exactly its own row; the subscriptions-on acceptance has
    /// its own assert.
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
            export_controller: "",
            export_sink: "s3",
            subscriptions_enabled: false,
            subscriptions_fanout: "",
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
            bulk_export_enabled: true,
            bulk_export_output_backend: "local-fs",
            bulk_submit_enabled: true,
            bulk_submit_output_backend: "local-fs",
            audit_backend: AuditBackend::File,
            sof_enabled: true,
            export_controller: "memory",
            export_sink: "fs",
            subscriptions_enabled: true,
            subscriptions_fanout: "memory",
        };
        assert_eq!(validate_cluster_config(&view), Ok(()));
    }

    #[test]
    fn cluster_on_accepts_a_fully_shared_configuration() {
        assert_eq!(validate_cluster_config(&safe_cluster_view()), Ok(()));
        // Subscriptions on a postgres primary (fanout unset → pg-notify) is
        // the supported cluster shape.
        let with_subscriptions = ClusterConfigView {
            subscriptions_enabled: true,
            ..safe_cluster_view()
        };
        assert_eq!(validate_cluster_config(&with_subscriptions), Ok(()));
        let explicit_fanout = ClusterConfigView {
            subscriptions_enabled: true,
            subscriptions_fanout: "pg-notify",
            ..safe_cluster_view()
        };
        assert_eq!(validate_cluster_config(&explicit_fanout), Ok(()));
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
            Case {
                name: "A1 explicit memory export controller",
                view: ClusterConfigView {
                    export_controller: "memory",
                    ..safe_cluster_view()
                },
                expect_var: "HFS_EXPORT_CONTROLLER",
            },
            Case {
                name: "A1 node-local export sink",
                view: ClusterConfigView {
                    export_sink: "fs",
                    ..safe_cluster_view()
                },
                expect_var: "HFS_EXPORT_SINK",
            },
            Case {
                name: "B explicit memory subscriptions fanout",
                view: ClusterConfigView {
                    subscriptions_enabled: true,
                    subscriptions_fanout: "memory",
                    ..safe_cluster_view()
                },
                expect_var: "HFS_SUBSCRIPTIONS_FANOUT",
            },
            Case {
                name: "B subscriptions on a non-postgres primary",
                view: ClusterConfigView {
                    subscriptions_enabled: true,
                    primary_backend: BackendKind::MongoDB,
                    ..safe_cluster_view()
                },
                expect_var: "HFS_SUBSCRIPTIONS_ENABLED",
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
            bulk_export_enabled: true,
            bulk_export_output_backend: "local-fs",
            bulk_submit_enabled: true,
            bulk_submit_output_backend: "local-fs",
            audit_backend: AuditBackend::File,
            sof_enabled: true,
            export_controller: "memory",
            export_sink: "fs",
            subscriptions_enabled: true,
            subscriptions_fanout: "memory",
        };
        let errors = validate_cluster_config(&view).unwrap_err();
        // 7 pre-Phase-3 rows + the memory fanout + the non-postgres primary.
        assert_eq!(errors.len(), 9);
    }

    /// The C2 resolution table (warn-only by design — contrast with the
    /// refusal rows above).
    #[test]
    fn jwks_coordination_resolution_table() {
        use JwksCoordinationMode::*;

        // Unset follows the cluster switch.
        assert_eq!(
            resolve_jwks_coordination("", false, None),
            Ok((Local, None))
        );
        assert_eq!(
            resolve_jwks_coordination("", true, None),
            Ok((Database, None))
        );

        // Explicit local: allowed everywhere; warned (not refused) under
        // cluster.
        assert_eq!(
            resolve_jwks_coordination("local", false, None),
            Ok((Local, None))
        );
        let (mode, warning) = resolve_jwks_coordination("local", true, None).unwrap();
        assert_eq!(mode, Local);
        assert!(
            warning
                .as_deref()
                .is_some_and(|w| w.contains("HFS_AUTH_JWKS_COORDINATION")),
            "the warning must name the variable"
        );

        // Explicit database: fine with or without the cluster switch.
        assert_eq!(
            resolve_jwks_coordination("database", false, None),
            Ok((Database, None))
        );
        assert_eq!(
            resolve_jwks_coordination("database", true, None),
            Ok((Database, None))
        );

        // Redis needs the feature and a URL; both errors name the variable.
        let no_url = resolve_jwks_coordination("redis", true, None).unwrap_err();
        assert!(no_url.contains("HFS_AUTH_JWKS_COORDINATION"));
        let empty_url = resolve_jwks_coordination("redis", true, Some("")).unwrap_err();
        assert!(empty_url.contains("HFS_AUTH_JWKS_COORDINATION"));
        if cfg!(feature = "redis") {
            assert_eq!(
                resolve_jwks_coordination("redis", true, Some("redis://localhost:6379")),
                Ok((Redis, None))
            );
        } else {
            let no_feature =
                resolve_jwks_coordination("redis", true, Some("redis://localhost:6379"))
                    .unwrap_err();
            assert!(no_feature.contains("'redis' feature"));
        }

        // Unknown values fail fast, naming the variable.
        let unknown = resolve_jwks_coordination("zookeeper", true, None).unwrap_err();
        assert!(unknown.contains("HFS_AUTH_JWKS_COORDINATION"));
    }

    /// The E1 resolution table (warn-only by design — contrast with the
    /// refusal rows above, mirroring C2's `jwks_coordination_resolution_table`).
    #[test]
    fn composite_sync_durability_resolution_table() {
        // Not clustered: never warns, regardless of backend/mode.
        assert_eq!(
            resolve_composite_sync_durability(false, BackendKind::MongoDB, true, false),
            None
        );
        // No composite secondary: nothing to warn about.
        assert_eq!(
            resolve_composite_sync_durability(true, BackendKind::MongoDB, false, false),
            None
        );
        // Synchronous mode is already durable by blocking — no warning even
        // on a non-Postgres primary.
        assert_eq!(
            resolve_composite_sync_durability(true, BackendKind::MongoDB, true, true),
            None
        );
        // Postgres primary: the outbox is wired, so no warning.
        assert_eq!(
            resolve_composite_sync_durability(true, BackendKind::Postgres, true, false),
            None
        );
        // The only warning case: clustered, async/hybrid mode, a composite
        // secondary, and a non-Postgres primary.
        let warning =
            resolve_composite_sync_durability(true, BackendKind::MongoDB, true, false).unwrap();
        assert!(warning.contains("non-postgres primary"));
    }

    #[test]
    fn disabled_subsystems_are_exempt() {
        // A disabled subsystem's unsafe backend cannot hurt a cluster:
        // auth off + memory jti, bulk off + local-fs, audit off (None).
        let view = ClusterConfigView {
            bulk_export_enabled: false,
            bulk_export_output_backend: "local-fs",
            bulk_submit_enabled: false,
            bulk_submit_output_backend: "local-fs",
            audit_backend: AuditBackend::None,
            sof_enabled: false,
            export_controller: "memory",
            export_sink: "fs",
            subscriptions_enabled: false,
            subscriptions_fanout: "memory",
            ..safe_cluster_view()
        };
        assert_eq!(validate_cluster_config(&view), Ok(()));
    }
}
