//! Configuration types for the S3 backend.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::error::{BackendError, StorageError, StorageResult};
use crate::tenant::SYSTEM_TENANT;

/// Tenant-to-bucket resolution for S3.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum S3TenancyMode {
    /// All tenants share one bucket with tenant-specific key prefixes.
    PrefixPerTenant {
        /// Shared bucket name.
        bucket: String,
    },

    /// Each tenant maps to a specific bucket.
    ///
    /// The system tenant can use `default_system_bucket`.
    BucketPerTenant {
        /// Explicit tenant -> bucket map.
        tenant_bucket_map: HashMap<String, String>,
        /// Optional fallback bucket for `__system__` tenant.
        default_system_bucket: Option<String>,
    },
}

/// Configuration for the AWS S3 backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3BackendConfig {
    /// How tenant data is mapped to buckets/prefixes.
    pub tenancy_mode: S3TenancyMode,

    /// Optional global key prefix applied before backend keys.
    pub prefix: Option<String>,

    /// AWS region override (falls back to provider chain if unset).
    pub region: Option<String>,

    /// Optional S3-compatible endpoint URL (for example, MinIO).
    ///
    /// When unset, the backend uses normal AWS S3 endpoint resolution.
    pub endpoint_url: Option<String>,

    /// Force path-style bucket addressing.
    ///
    /// In S3-compatible endpoint mode this may be defaulted at runtime.
    /// In AWS mode (`endpoint_url == None`), defaults preserve current behavior.
    #[serde(default)]
    pub force_path_style: bool,

    /// Allow insecure HTTP endpoint URLs.
    ///
    /// This only matters when `endpoint_url` is set. AWS mode is unaffected.
    #[serde(default)]
    pub allow_http: bool,

    /// Validate all configured buckets on startup with `HeadBucket`.
    pub validate_buckets_on_startup: bool,

    /// Max NDJSON lines per export output part.
    pub bulk_export_part_size: u32,

    /// Default ingestion batch size for bulk submit processing.
    pub bulk_submit_batch_size: u32,
}

impl Default for S3BackendConfig {
    fn default() -> Self {
        Self {
            tenancy_mode: S3TenancyMode::PrefixPerTenant {
                bucket: "hfs".to_string(),
            },
            prefix: None,
            region: None,
            endpoint_url: None,
            force_path_style: false,
            allow_http: false,
            validate_buckets_on_startup: true,
            bulk_export_part_size: 10_000,
            bulk_submit_batch_size: 100,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_config() -> S3BackendConfig {
        S3BackendConfig {
            tenancy_mode: S3TenancyMode::PrefixPerTenant {
                bucket: "test-bucket".to_string(),
            },
            ..Default::default()
        }
    }

    #[test]
    fn validate_accepts_https_endpoint_without_allow_http() {
        let mut config = base_config();
        config.endpoint_url = Some("https://minio.example.local:9000".to_string());

        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_rejects_http_endpoint_when_allow_http_false() {
        let mut config = base_config();
        config.endpoint_url = Some("http://127.0.0.1:9000".to_string());
        config.allow_http = false;

        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_accepts_http_endpoint_when_allow_http_true() {
        let mut config = base_config();
        config.endpoint_url = Some("http://127.0.0.1:9000".to_string());
        config.allow_http = true;

        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_rejects_malformed_endpoint_scheme() {
        let mut config = base_config();
        config.endpoint_url = Some("ftp://minio.local:9000".to_string());
        config.allow_http = true;

        assert!(config.validate().is_err());
    }

    fn bucket_per_tenant(
        pairs: &[(&str, &str)],
        default_system_bucket: Option<&str>,
    ) -> S3BackendConfig {
        S3BackendConfig {
            tenancy_mode: S3TenancyMode::BucketPerTenant {
                tenant_bucket_map: pairs
                    .iter()
                    .map(|(t, b)| (t.to_string(), b.to_string()))
                    .collect(),
                default_system_bucket: default_system_bucket.map(str::to_string),
            },
            ..Default::default()
        }
    }

    fn message(config: &S3BackendConfig) -> String {
        match config.validate() {
            Err(StorageError::Backend(BackendError::Internal { message, .. })) => message,
            other => panic!("expected a backend-internal validation error, got {other:?}"),
        }
    }

    #[test]
    fn bucket_per_tenant_accepts_distinct_buckets() {
        let config = bucket_per_tenant(
            &[("acme", "acme-bucket"), ("globex", "globex-bucket")],
            Some("system-bucket"),
        );
        assert!(config.validate().is_ok());
    }

    /// The whole isolation guarantee of this mode. `tenant_location` gives
    /// `BucketPerTenant` an un-prefixed keyspace, so two tenants in one bucket
    /// write the *same* key for the same resource id and silently clobber each
    /// other — and the mode's declared `DatabasePerTenant` capability would be a
    /// lie. Nothing downstream can detect it, so it has to be rejected here.
    #[test]
    fn bucket_per_tenant_rejects_two_tenants_sharing_a_bucket() {
        let config = bucket_per_tenant(&[("acme", "shared"), ("globex", "shared")], None);
        let message = message(&config);
        assert!(
            message.contains("distinct bucket per tenant"),
            "unexpected message: {message}"
        );
        assert!(
            message.contains("shared by acme, globex"),
            "message must name the colliding tenants: {message}"
        );
    }

    /// `HashMap` iteration order is nondeterministic, so the enumeration in the
    /// message is sorted. An operator-facing error that reads differently on
    /// every boot cannot be matched, diffed, or searched for.
    #[test]
    fn bucket_per_tenant_collision_message_is_deterministic() {
        let config = bucket_per_tenant(
            &[
                ("zeta", "one"),
                ("alpha", "one"),
                ("mu", "two"),
                ("beta", "two"),
            ],
            None,
        );
        let first = message(&config);
        for _ in 0..16 {
            assert_eq!(first, message(&config));
        }
        let one = first.find("one shared by").expect("bucket `one` reported");
        let two = first.find("two shared by").expect("bucket `two` reported");
        assert!(one < two, "buckets must be listed in sorted order: {first}");
        assert!(first.contains("one shared by alpha, zeta"), "{first}");
        assert!(first.contains("two shared by beta, mu"), "{first}");
    }

    /// `tenant_location` routes `__system__` to `default_system_bucket` when it
    /// has no explicit mapping, so that bucket is a tenant bucket like any other
    /// and must not be shared.
    #[test]
    fn bucket_per_tenant_rejects_default_system_bucket_shared_with_a_tenant() {
        let config = bucket_per_tenant(&[("acme", "shared")], Some("shared"));
        let message = message(&config);
        assert!(
            message.contains("shared by __system__, acme"),
            "unexpected message: {message}"
        );
    }

    /// The converse: with `__system__` mapped explicitly, `default_system_bucket`
    /// only ever holds cross-tenant control-plane state (`tenants/`,
    /// `_system.user-settings/`), which is structurally disjoint from the
    /// `resources/`, `history/`, and `bulk/` prefixes tenant data uses. Sharing
    /// it is not a collision, and rejecting it would break working deployments.
    #[test]
    fn bucket_per_tenant_allows_registry_bucket_to_double_as_a_tenant_bucket() {
        let config = bucket_per_tenant(
            &[("acme", "acme-bucket"), (SYSTEM_TENANT, "system-bucket")],
            Some("acme-bucket"),
        );
        assert!(config.validate().is_ok(), "{:?}", config.validate());
    }

    /// Whitespace around a bucket name cannot make two tenants look distinct:
    /// a padded name is not a legal S3 bucket name, so both would resolve to
    /// nothing rather than to different buckets.
    #[test]
    fn bucket_per_tenant_compares_buckets_after_trimming() {
        let config = bucket_per_tenant(&[("acme", "shared"), ("globex", " shared ")], None);
        assert!(config.validate().is_err());
    }
}

impl S3BackendConfig {
    /// Validates configuration invariants.
    pub fn validate(&self) -> StorageResult<()> {
        if self.bulk_export_part_size == 0 {
            return Err(StorageError::Backend(BackendError::Internal {
                backend_name: "s3".to_string(),
                message: "bulk_export_part_size must be > 0".to_string(),
                source: None,
            }));
        }

        if self.bulk_submit_batch_size == 0 {
            return Err(StorageError::Backend(BackendError::Internal {
                backend_name: "s3".to_string(),
                message: "bulk_submit_batch_size must be > 0".to_string(),
                source: None,
            }));
        }

        if let Some(endpoint_url) = self.endpoint_url.as_deref() {
            let endpoint_url = endpoint_url.trim();
            if endpoint_url.is_empty() {
                return Err(StorageError::Backend(BackendError::Internal {
                    backend_name: "s3".to_string(),
                    message: "endpoint_url must not be empty when provided".to_string(),
                    source: None,
                }));
            }

            let lower = endpoint_url.to_ascii_lowercase();
            let is_http = lower.starts_with("http://");
            let is_https = lower.starts_with("https://");
            if !is_http && !is_https {
                return Err(StorageError::Backend(BackendError::Internal {
                    backend_name: "s3".to_string(),
                    message: "endpoint_url must start with http:// or https://".to_string(),
                    source: None,
                }));
            }

            if is_http && !self.allow_http {
                return Err(StorageError::Backend(BackendError::Internal {
                    backend_name: "s3".to_string(),
                    message: "http endpoint_url requires allow_http=true".to_string(),
                    source: None,
                }));
            }
        }

        match &self.tenancy_mode {
            S3TenancyMode::PrefixPerTenant { bucket } => {
                if bucket.trim().is_empty() {
                    return Err(StorageError::Backend(BackendError::Internal {
                        backend_name: "s3".to_string(),
                        message: "prefix-per-tenant bucket must not be empty".to_string(),
                        source: None,
                    }));
                }
            }
            S3TenancyMode::BucketPerTenant {
                tenant_bucket_map,
                default_system_bucket,
            } => {
                if tenant_bucket_map.is_empty() && default_system_bucket.is_none() {
                    return Err(StorageError::Backend(BackendError::Internal {
                        backend_name: "s3".to_string(),
                        message: "bucket-per-tenant requires at least one mapped bucket or default_system_bucket"
                            .to_string(),
                        source: None,
                    }));
                }

                if tenant_bucket_map.values().any(|b| b.trim().is_empty()) {
                    return Err(StorageError::Backend(BackendError::Internal {
                        backend_name: "s3".to_string(),
                        message: "bucket-per-tenant mapping contains empty bucket name".to_string(),
                        source: None,
                    }));
                }

                if default_system_bucket
                    .as_ref()
                    .map(|b| b.trim().is_empty())
                    .unwrap_or(false)
                {
                    return Err(StorageError::Backend(BackendError::Internal {
                        backend_name: "s3".to_string(),
                        message: "default_system_bucket must not be empty when provided"
                            .to_string(),
                        source: None,
                    }));
                }

                Self::validate_distinct_buckets(tenant_bucket_map, default_system_bucket.as_ref())?;
            }
        }

        Ok(())
    }

    /// Rejects a `BucketPerTenant` mapping in which two tenants resolve to the
    /// same bucket.
    ///
    /// # Why this is a hard error and not a warning
    ///
    /// The bucket is the *only* thing separating tenants in this mode.
    /// `S3Backend::tenant_location` builds `S3Keyspace::new(global_prefix)` for
    /// `BucketPerTenant` — with no `with_tenant_prefix`, unlike
    /// `PrefixPerTenant`. So two tenants sharing a bucket do not merely
    /// sit next to each other: they write **byte-identical keys**
    /// (`resources/Patient/123/current.json`) and silently overwrite, delete,
    /// and read back each other's resources. There is no downstream check that
    /// would catch it, because at that point the two tenants are indistinguishable.
    ///
    /// It is also the exact condition under which this mode's declared
    /// [`BackendCapability::DatabasePerTenant`](crate::core::BackendCapability::DatabasePerTenant)
    /// — "each tenant gets a distinct physical database or storage container" —
    /// would be false. Validating here is what makes that declaration true by
    /// construction rather than by convention.
    ///
    /// # System tenant
    ///
    /// `default_system_bucket` is folded in as `__system__`'s bucket only when
    /// the map has no explicit `__system__` entry, mirroring the fallback in
    /// `tenant_location`. When `__system__` *is* mapped explicitly, the default
    /// bucket only ever serves cross-tenant control-plane state (the tenant
    /// registry under `tenants/` and user settings under
    /// `_system.user-settings/`), which is structurally disjoint from the
    /// `resources/`, `history/`, and `bulk/` prefixes tenant data uses — so
    /// sharing it with a tenant's bucket is not a collision and is not rejected.
    ///
    /// Buckets are compared on their trimmed value: a name with surrounding
    /// whitespace is not a legal S3 bucket name, so trimming can only reject
    /// configurations that were already broken.
    fn validate_distinct_buckets(
        tenant_bucket_map: &HashMap<String, String>,
        default_system_bucket: Option<&String>,
    ) -> StorageResult<()> {
        // BTree* rather than Hash*: the error message enumerates the offending
        // config, and `HashMap` iteration order is nondeterministic. An error
        // that reads differently on every boot is not a diagnosable one.
        let mut by_bucket: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for (tenant_id, bucket) in tenant_bucket_map {
            by_bucket
                .entry(bucket.trim())
                .or_default()
                .insert(tenant_id.as_str());
        }
        if let Some(bucket) = default_system_bucket {
            if !tenant_bucket_map.contains_key(SYSTEM_TENANT) {
                by_bucket
                    .entry(bucket.trim())
                    .or_default()
                    .insert(SYSTEM_TENANT);
            }
        }

        let shared: Vec<String> = by_bucket
            .into_iter()
            .filter(|(_, tenants)| tenants.len() > 1)
            .map(|(bucket, tenants)| {
                format!(
                    "{} shared by {}",
                    bucket,
                    tenants.into_iter().collect::<Vec<_>>().join(", ")
                )
            })
            .collect();

        if !shared.is_empty() {
            return Err(StorageError::Backend(BackendError::Internal {
                backend_name: "s3".to_string(),
                message: format!(
                    "bucket-per-tenant requires a distinct bucket per tenant, \
                     because tenants sharing a bucket write identical keys and \
                     overwrite each other: {}",
                    shared.join("; ")
                ),
                source: None,
            }));
        }

        Ok(())
    }

    /// Returns a de-duplicated set of all buckets referenced by this config.
    pub fn configured_buckets(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        match &self.tenancy_mode {
            S3TenancyMode::PrefixPerTenant { bucket } => {
                out.insert(bucket.clone());
            }
            S3TenancyMode::BucketPerTenant {
                tenant_bucket_map,
                default_system_bucket,
            } => {
                out.extend(tenant_bucket_map.values().cloned());
                if let Some(bucket) = default_system_bucket {
                    out.insert(bucket.clone());
                }
            }
        }
        out
    }
}
