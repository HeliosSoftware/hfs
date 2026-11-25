//! The `HFS_*` environment variables some Helios component reads, so that
//! startup can warn about one that is set but read by nothing (#2707). A
//! misspelt or retired name leaves the setting it meant at its default, and
//! for a security setting such as the tenant claim that default can be the
//! unsafe one.

/// Prefixes of the variables test harnesses and scripts read, which no server
/// does.
const HARNESS_PREFIXES: [&str; 3] = ["HFS_TEST_", "HFS_E2E_", "HFS_BENCH_"];

/// Every `HFS_*` variable a crate of this workspace reads, in byte order. The
/// test `the_list_is_every_variable_the_sources_name` keeps it equal to the
/// quoted `"HFS_…"` literals under `crates/*/src` and in `crates/*/build.rs`,
/// harness variables aside.
const KNOWN: &[&str] = &[
    "HFS_AUDIT_BACKEND",
    "HFS_AUDIT_CLOUDWATCH_LOG_GROUP",
    "HFS_AUDIT_CLOUDWATCH_LOG_STREAM",
    "HFS_AUDIT_CLOUDWATCH_REGION",
    "HFS_AUDIT_DATABASE_URL",
    "HFS_AUDIT_EXCLUDE_PATHS",
    "HFS_AUDIT_FILE_PATH",
    "HFS_AUDIT_MONGODB_DATABASE",
    "HFS_AUDIT_OVERFLOW",
    "HFS_AUDIT_QUEUE_CAPACITY",
    "HFS_AUDIT_S3_BUCKET",
    "HFS_AUDIT_S3_PREFIX",
    "HFS_AUDIT_S3_REGION",
    "HFS_AUDIT_S3_VALIDATE_BUCKETS",
    "HFS_AUDIT_SOURCE_OBSERVER",
    "HFS_AUDIT_WRITERS",
    "HFS_AUDIT_WRITE_RETRIES",
    "HFS_AUTH_ALGORITHMS",
    "HFS_AUTH_AUDIENCE",
    "HFS_AUTH_ENABLED",
    "HFS_AUTH_ENCOUNTER_CLAIM",
    "HFS_AUTH_FHIR_USER_CLAIM",
    "HFS_AUTH_ISSUER",
    "HFS_AUTH_JWKS_MIN_REFRESH_INTERVAL",
    "HFS_AUTH_JWKS_URL",
    "HFS_AUTH_PATIENT_CLAIM",
    "HFS_AUTH_TENANT_CLAIM",
    "HFS_BASE_URL",
    "HFS_BATCH_MAX_CONCURRENCY",
    "HFS_BUILD_GIT_SHA",
    "HFS_BUILD_VERSION_SUFFIX",
    "HFS_BULK_EXPORT_BATCH_SIZE",
    "HFS_BULK_EXPORT_CLEANUP_INTERVAL",
    "HFS_BULK_EXPORT_DISABLE_LOCAL_WORKER",
    "HFS_BULK_EXPORT_ENABLED",
    "HFS_BULK_EXPORT_FILE_URL_TTL",
    "HFS_BULK_EXPORT_HEARTBEAT_INTERVAL",
    "HFS_BULK_EXPORT_LEASE_DURATION",
    "HFS_BULK_EXPORT_MAX_ATTEMPTS",
    "HFS_BULK_EXPORT_MAX_CONCURRENT_PER_TENANT",
    "HFS_BULK_EXPORT_OUTPUT_BACKEND",
    "HFS_BULK_EXPORT_OUTPUT_DIR",
    "HFS_BULK_EXPORT_OUTPUT_TTL",
    "HFS_BULK_EXPORT_REQUIRES_ACCESS_TOKEN",
    "HFS_BULK_EXPORT_S3_BUCKET",
    "HFS_BULK_EXPORT_S3_ENDPOINT",
    "HFS_BULK_EXPORT_S3_FORCE_PATH_STYLE",
    "HFS_BULK_EXPORT_S3_REGION",
    "HFS_BULK_EXPORT_SINCE_NEWLY_ADDED",
    "HFS_BULK_EXPORT_WORKER_CONCURRENCY",
    "HFS_BULK_SUBMIT_BATCH_SIZE",
    "HFS_BULK_SUBMIT_BATCH_WRITERS",
    "HFS_BULK_SUBMIT_BLOCK_CONCURRENT_SUBMISSION",
    "HFS_BULK_SUBMIT_BULK_INDEX_REBUILD",
    "HFS_BULK_SUBMIT_CLEANUP_INTERVAL",
    "HFS_BULK_SUBMIT_CLIENT_ID",
    "HFS_BULK_SUBMIT_DECRYPTION_KEY",
    "HFS_BULK_SUBMIT_DEFER_INDEXING",
    "HFS_BULK_SUBMIT_DISABLE_LOCAL_WORKER",
    "HFS_BULK_SUBMIT_ENABLED",
    "HFS_BULK_SUBMIT_FETCH_READ_TIMEOUT",
    "HFS_BULK_SUBMIT_FILE_CONCURRENCY",
    "HFS_BULK_SUBMIT_FILE_URL_TTL",
    "HFS_BULK_SUBMIT_HEARTBEAT_INTERVAL",
    "HFS_BULK_SUBMIT_INDEX_COALESCE",
    "HFS_BULK_SUBMIT_INDEX_CONCURRENCY",
    "HFS_BULK_SUBMIT_INDEX_DURING_INGEST",
    "HFS_BULK_SUBMIT_INDEX_MAX_WAIT",
    "HFS_BULK_SUBMIT_INDEX_PAGE_BYTES",
    "HFS_BULK_SUBMIT_INDEX_QUEUE",
    "HFS_BULK_SUBMIT_LEASE_DURATION",
    "HFS_BULK_SUBMIT_MANIFEST_FETCH_TIMEOUT",
    "HFS_BULK_SUBMIT_MANIFEST_PAGE_SIZE",
    "HFS_BULK_SUBMIT_MAX_CONCURRENT_PER_TENANT",
    "HFS_BULK_SUBMIT_MAX_ENCRYPTED_FILE_BYTES",
    "HFS_BULK_SUBMIT_MAX_LINE_BYTES",
    "HFS_BULK_SUBMIT_MAX_MANIFEST_BYTES",
    "HFS_BULK_SUBMIT_OUTBOUND_SCOPE",
    "HFS_BULK_SUBMIT_OUTPUT_BACKEND",
    "HFS_BULK_SUBMIT_OUTPUT_DIR",
    "HFS_BULK_SUBMIT_OUTPUT_TTL",
    "HFS_BULK_SUBMIT_POLL_RATE_LIMIT",
    "HFS_BULK_SUBMIT_POLL_RATE_WINDOW",
    "HFS_BULK_SUBMIT_PRE_INGEST_RETRY_AFTER",
    "HFS_BULK_SUBMIT_PRIVATE_KEY",
    "HFS_BULK_SUBMIT_REINDEX_IDS_MAX",
    "HFS_BULK_SUBMIT_REINDEX_SINCE_MARGIN",
    "HFS_BULK_SUBMIT_REQUIRES_ACCESS_TOKEN",
    "HFS_BULK_SUBMIT_RETRY_AFTER",
    "HFS_BULK_SUBMIT_S3_BUCKET",
    "HFS_BULK_SUBMIT_S3_ENDPOINT",
    "HFS_BULK_SUBMIT_S3_FORCE_PATH_STYLE",
    "HFS_BULK_SUBMIT_S3_INGEST_CONCURRENCY",
    "HFS_BULK_SUBMIT_S3_REGION",
    "HFS_BULK_SUBMIT_SIGNING_ALG",
    "HFS_BULK_SUBMIT_SKIP_UNCHANGED",
    "HFS_BULK_SUBMIT_TRUSTED_PROVIDERS",
    "HFS_BULK_SUBMIT_WORKER_CONCURRENCY",
    "HFS_COMPOSITE_SYNC_MODE",
    "HFS_COMPOSITE_SYNC_REPAIR_BATCH",
    "HFS_COMPOSITE_SYNC_REPAIR_INTERVAL",
    "HFS_COMPOSITE_SYNC_SHUTDOWN_TIMEOUT",
    "HFS_CORS_HEADERS",
    "HFS_CORS_METHODS",
    "HFS_CORS_ORIGINS",
    "HFS_DASHBOARD_IDLE_REFRESH_SECS",
    "HFS_DASHBOARD_RECONCILE_SECS",
    "HFS_DASHBOARD_REFRESH_SECS",
    "HFS_DATABASE_URL",
    "HFS_DATA_DIR",
    "HFS_DEFAULT_FHIR_VERSION",
    "HFS_DEFAULT_PAGE_SIZE",
    "HFS_DEFAULT_TENANT",
    "HFS_ELASTICSEARCH_BULK_CONCURRENCY",
    "HFS_ELASTICSEARCH_BULK_MAX_BYTES",
    "HFS_ELASTICSEARCH_CONTAINED_SWEEP_CONCURRENCY",
    "HFS_ELASTICSEARCH_INDEX_PREFIX",
    "HFS_ELASTICSEARCH_MAX_TERMS_COUNT",
    "HFS_ELASTICSEARCH_NESTED_OBJECTS_LIMIT",
    "HFS_ELASTICSEARCH_NODES",
    "HFS_ELASTICSEARCH_PASSWORD",
    "HFS_ELASTICSEARCH_REFRESH_INTERVAL",
    "HFS_ELASTICSEARCH_REINDEX_PREPARE_THREADS",
    "HFS_ELASTICSEARCH_REINDEX_REFRESH",
    "HFS_ELASTICSEARCH_REQUEST_TIMEOUT_MS",
    "HFS_ELASTICSEARCH_USERNAME",
    "HFS_ELASTICSEARCH_WRITE_REFRESH",
    "HFS_ENABLE_CORS",
    "HFS_ENABLE_REQUEST_ID",
    "HFS_ENABLE_VERSIONING",
    "HFS_EVERYTHING_MAX_UNPAGED",
    "HFS_EXPERIMENT_CACHE_KB",
    "HFS_EXPORT_CLEANUP_INTERVAL",
    "HFS_EXPORT_CONTROLLER",
    "HFS_EXPORT_DIR",
    "HFS_EXPORT_MAX_CONCURRENCY",
    "HFS_EXPORT_MAX_JOBS_PER_TENANT",
    "HFS_EXPORT_OUTPUT_TTL",
    "HFS_EXPORT_PRESIGN_TTL_SECS",
    "HFS_EXPORT_S3_BUCKET",
    "HFS_EXPORT_S3_ENDPOINT",
    "HFS_EXPORT_S3_FORCE_PATH_STYLE",
    "HFS_EXPORT_S3_REGION",
    "HFS_EXPORT_SHARD_ROWS",
    "HFS_EXPORT_SINK",
    "HFS_GIT_SHA",
    "HFS_INDEX_THREADS",
    "HFS_JWT_TENANT_CLAIM",
    "HFS_LOG_LEVEL",
    "HFS_MAX_BODY_SIZE",
    "HFS_MAX_PAGE_SIZE",
    "HFS_MONGODB",
    "HFS_MONGODB_ADAPTIVE_OFFSET_PAGING",
    "HFS_MONGODB_BROAD_SEARCH_CONCURRENCY",
    "HFS_MONGODB_BROAD_TOTAL_CONCURRENCY",
    "HFS_MONGODB_BULK_INGEST_INDEX_CONCURRENCY",
    "HFS_MONGODB_CONNECT_TIMEOUT_MS",
    "HFS_MONGODB_COUNT_BY_TENANT_MAX_TIME_MS",
    "HFS_MONGODB_DATABASE",
    "HFS_MONGODB_DEFERRED_INDEX_REPAIR_INTERVAL",
    "HFS_MONGODB_INDEX_BUILD",
    "HFS_MONGODB_MAX_CONCURRENT_TRANSACTION_BUNDLES",
    "HFS_MONGODB_MAX_CONNECTIONS",
    "HFS_MONGODB_MAX_INCLUDED_RESOURCES",
    "HFS_MONGODB_MISSING_METADATA",
    "HFS_MONGODB_PROBE_TIMEOUT_MS",
    "HFS_MONGODB_REINDEX_OVERLAP",
    "HFS_MONGODB_REINDEX_PREFETCH",
    "HFS_MONGODB_REINDEX_PREPARE_THREADS",
    "HFS_MONGODB_SEARCH_TIMEOUT_MS",
    "HFS_MONGODB_SERVER_SELECTION_TIMEOUT_MS",
    "HFS_MONGODB_SOF_CURSOR_BATCH_SIZE",
    "HFS_MONGODB_TRANSACTION_BUNDLE_DEFERRED_INDEX_ENTRIES",
    "HFS_MONGODB_TRANSACTION_BUNDLE_MAX_PAYLOAD_BYTES",
    "HFS_MONGODB_TRANSACTION_BUNDLE_WEIGHT_ENTRIES",
    "HFS_MONGODB_URI",
    "HFS_MONGODB_URL",
    "HFS_NL_SEARCH_API_KEY",
    "HFS_NL_SEARCH_BASE_URL",
    "HFS_NL_SEARCH_DAILY_LIMIT",
    "HFS_NL_SEARCH_ENABLED",
    "HFS_NL_SEARCH_MAX_CHARS",
    "HFS_NL_SEARCH_MODEL",
    "HFS_NL_SEARCH_RATE_LIMIT",
    "HFS_NL_SEARCH_RATE_WINDOW_SECS",
    "HFS_OUTBOUND_BEARER_TOKEN",
    "HFS_PERF_PHASES",
    "HFS_PG_BULK_INDEX_BUILD_CONCURRENCY",
    "HFS_PG_BULK_INDEX_MAINTENANCE_WORK_MEM",
    "HFS_PG_CONNECT_TIMEOUT_SECS",
    "HFS_PG_COUNT_BY_TENANT_STATEMENT_TIMEOUT_MS",
    "HFS_PG_DBNAME",
    "HFS_PG_ELIGIBLE_CACHE",
    "HFS_PG_HOST",
    "HFS_PG_MAX_CONNECTIONS",
    "HFS_PG_PASSWORD",
    "HFS_PG_PLAN_CACHE_MODE",
    "HFS_PG_POOL_WAIT_TIMEOUT_SECS",
    "HFS_PG_PORT",
    "HFS_PG_SP_FINGERPRINT_TTL_MS",
    "HFS_PG_SSL_CERT",
    "HFS_PG_SSL_KEY",
    "HFS_PG_SSL_MODE",
    "HFS_PG_SSL_ROOT_CERT",
    "HFS_PG_STATEMENT_CACHE_MAX",
    "HFS_PG_STATEMENT_TIMEOUT_MS",
    "HFS_PG_USER",
    "HFS_POSTGRES_REINDEX_WIDTH",
    "HFS_PROFILE_BUNDLES",
    "HFS_PROFILE_CORPUS",
    "HFS_RATE_LIMIT_IPV6_PREFIX",
    "HFS_REINDEX_BATCH_BYTES",
    "HFS_REINDEX_BATCH_SIZE",
    "HFS_REINDEX_PREFETCH",
    "HFS_REINDEX_STREAMS",
    "HFS_REQUEST_TIMEOUT",
    "HFS_REQUIRE_IF_MATCH",
    "HFS_RETURN_GONE",
    "HFS_S3_ALLOW_HTTP",
    "HFS_S3_BUCKET",
    "HFS_S3_ENDPOINT",
    "HFS_S3_FORCE_PATH_STYLE",
    "HFS_S3_KEYSPACE_LAYOUT",
    "HFS_S3_PREFIX",
    "HFS_S3_REGION",
    "HFS_S3_SCAN_CONCURRENCY",
    "HFS_S3_SYSTEM_BUCKET",
    "HFS_S3_TENANCY",
    "HFS_S3_TENANT_BUCKETS",
    "HFS_S3_VALIDATE_BUCKETS",
    "HFS_SEARCH_PARAM_CACHE_MAX_TENANTS",
    "HFS_SEARCH_PARAM_CACHE_TTL",
    "HFS_SEED_CONFORMANCE",
    "HFS_SERVER_HOST",
    "HFS_SERVER_PORT",
    "HFS_SMART_AUTHORIZE_ENDPOINT",
    "HFS_SMART_END_SESSION_ENDPOINT",
    "HFS_SMART_INTROSPECTION_ENDPOINT",
    "HFS_SMART_JWKS_URL",
    "HFS_SMART_MANAGEMENT_ENDPOINT",
    "HFS_SMART_REGISTRATION_ENDPOINT",
    "HFS_SMART_REVOCATION_ENDPOINT",
    "HFS_SMART_TOKEN_ENDPOINT",
    "HFS_SOF_ENABLED",
    "HFS_SOF_RUNNER_HEADER",
    "HFS_SOF_SQLQUERY_MAX_ROWS",
    "HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD",
    "HFS_SOF_SQLQUERY_MAX_VDS",
    "HFS_SOF_SQLQUERY_TIMEOUT_SECS",
    "HFS_SQLITE_INDEX_BUILD_CACHE_KB",
    "HFS_SQLITE_INDEX_BUILD_THREADS",
    "HFS_SQLITE_SEARCH_PROBE",
    "HFS_STORAGE_BACKEND",
    "HFS_SUBSCRIPTIONS_ENABLED",
    "HFS_SUBSCRIPTION_ALLOW_PRIVATE_ENDPOINTS",
    "HFS_SUBSCRIPTION_DISPATCH_CONCURRENCY",
    "HFS_SUBSCRIPTION_EVENT_HISTORY_SIZE",
    "HFS_SUBSCRIPTION_EVENT_QUEUE_CAPACITY",
    "HFS_SUBSCRIPTION_HANDSHAKE_CONCURRENCY",
    "HFS_SUBSCRIPTION_HANDSHAKE_INITIAL_DELAY_MS",
    "HFS_SUBSCRIPTION_HANDSHAKE_MAX_ATTEMPTS",
    "HFS_SUBSCRIPTION_HANDSHAKE_RETRY_BASE_MS",
    "HFS_SUBSCRIPTION_HANDSHAKE_RETRY_MAX_MS",
    "HFS_SUBSCRIPTION_MAX_PENDING_NOTIFICATIONS",
    "HFS_SUBSCRIPTION_MAX_PENDING_NOTIFICATIONS_PER_TENANT",
    "HFS_SUBSCRIPTION_MESSAGE_SOURCE_ENDPOINT",
    "HFS_SUBSCRIPTION_MESSAGING_BEARER_TOKEN",
    "HFS_SUBSCRIPTION_MESSAGING_CREDENTIAL_HOSTS",
    "HFS_SUBSCRIPTION_MESSAGING_ENABLED",
    "HFS_SUBSCRIPTION_PERSIST_STATUS",
    "HFS_SUBSCRIPTION_PER_SUBSCRIPTION_QUEUE_CAPACITY",
    "HFS_SUBSCRIPTION_REHYDRATE",
    "HFS_SUBSCRIPTION_REHYDRATE_BATCH_SIZE",
    "HFS_SUBSCRIPTION_REHYDRATE_HANDSHAKE",
    "HFS_SUBSCRIPTION_REHYDRATE_HANDSHAKE_CONCURRENCY",
    "HFS_SUBSCRIPTION_SMTP_DEFAULT_SUBJECT",
    "HFS_SUBSCRIPTION_SMTP_ENCRYPTION",
    "HFS_SUBSCRIPTION_SMTP_FROM",
    "HFS_SUBSCRIPTION_SMTP_HOST",
    "HFS_SUBSCRIPTION_SMTP_PASSWORD",
    "HFS_SUBSCRIPTION_SMTP_PORT",
    "HFS_SUBSCRIPTION_SMTP_TIMEOUT_SECS",
    "HFS_SUBSCRIPTION_SMTP_USERNAME",
    "HFS_SUBSCRIPTION_STATUS_WRITE_TIMEOUT_MS",
    "HFS_TENANT_REQUIRE_PROVISIONED",
    "HFS_TENANT_ROUTING_MODE",
    "HFS_TENANT_STRICT_VALIDATION",
    "HFS_TERMINOLOGY_SERVER",
    "HFS_UI_ENABLED",
    "HFS_UI_LOGIN_CLIENT_ID",
    "HFS_UI_LOGIN_CLIENT_SECRET",
    "HFS_UI_LOGIN_COOKIE_SECURE",
    "HFS_UI_LOGIN_REDIRECT_URI",
    "HFS_UI_LOGIN_SCOPES",
    "HFS_UPDATE_SOF_GOLDEN",
    "HFS_VALIDATION_CONSTRAINTS",
    "HFS_VALIDATION_META_PROFILES",
    "HFS_VALIDATION_MODE",
    "HFS_VALIDATION_STORED_PROFILES",
    "HFS_VALIDATION_SUPPRESS_CONSTRAINTS",
    "HFS_VALIDATION_TERMINOLOGY",
    "HFS_VALIDATION_TERMINOLOGY_FAIL",
    "HFS_VALIDATION_TERMINOLOGY_TIMEOUT_MS",
    "HFS_VALIDATION_UNKNOWN_PROFILE",
    "HFS_WORKER_SHUTDOWN_TIMEOUT",
];

/// One warning for each name in `names` that starts with `HFS_`, is not
/// [`KNOWN`] and is not a harness variable, naming the closest known name
/// when one is a few edits away. Only names are read, never values.
pub(crate) fn unknown_variable_warnings<I, N>(names: I) -> Vec<String>
where
    I: IntoIterator<Item = N>,
    N: AsRef<str>,
{
    let mut unknown: Vec<String> = names
        .into_iter()
        .map(|name| name.as_ref().to_string())
        .filter(|name| {
            name.starts_with("HFS_")
                && !HARNESS_PREFIXES
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
                && KNOWN.binary_search(&name.as_str()).is_err()
        })
        .collect();
    unknown.sort();
    unknown.dedup();
    unknown
        .into_iter()
        .map(|name| match closest_known(&name) {
            Some(known) => {
                format!("{name} is set but no Helios component reads it; did you mean {known}?")
            }
            None => format!("{name} is set but no Helios component reads it"),
        })
        .collect()
}

/// The known name fewest edits away from `name`, when that is at most a
/// quarter of its length and at most 3; the first in byte order on a tie.
fn closest_known(name: &str) -> Option<&'static str> {
    let limit = (name.len() / 4).clamp(1, 3);
    KNOWN
        .iter()
        .map(|known| (edit_distance(name, known), *known))
        .filter(|(distance, _)| *distance <= limit)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, known)| known)
}

/// The Levenshtein distance between two names, by byte.
fn edit_distance(a: &str, b: &str) -> usize {
    let b = b.as_bytes();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, &left) in a.as_bytes().iter().enumerate() {
        let mut current = Vec::with_capacity(b.len() + 1);
        current.push(i + 1);
        for (j, &right) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(left != right);
            current.push(substitution.min(previous[j + 1] + 1).min(current[j] + 1));
        }
        previous = current;
    }
    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::Path;

    /// The quoted `"HFS_…"` literals in `text`.
    fn quoted_names(text: &str, names: &mut BTreeSet<String>) {
        for (at, _) in text.match_indices("\"HFS_") {
            let rest = &text[at + 1..];
            let end = rest
                .find(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
                .unwrap_or(rest.len());
            if rest[end..].starts_with('"') {
                names.insert(rest[..end].to_string());
            }
        }
    }

    fn scan(dir: &Path, names: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(dir).expect("read source dir") {
            let path = entry.expect("source entry").path();
            if path.is_dir() {
                scan(&path, names);
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && !path.ends_with(Path::new("hfs").join("src").join("env_names.rs"))
            {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    quoted_names(&text, names);
                }
            }
        }
    }

    /// The names the workspace sources read, this file and harness variables
    /// aside.
    fn source_names() -> BTreeSet<String> {
        let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates dir");
        let mut names = BTreeSet::new();
        for entry in std::fs::read_dir(crates).expect("read crates dir") {
            let krate = entry.expect("crate entry").path();
            if krate.join("src").is_dir() {
                scan(&krate.join("src"), &mut names);
            }
            if let Ok(text) = std::fs::read_to_string(krate.join("build.rs")) {
                quoted_names(&text, &mut names);
            }
        }
        names.retain(|name| {
            !HARNESS_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix))
        });
        names
    }

    #[test]
    fn the_list_is_every_variable_the_sources_name() {
        let sources = source_names();
        let listed: BTreeSet<String> = KNOWN.iter().map(|name| name.to_string()).collect();
        let missing: Vec<_> = sources.difference(&listed).collect();
        let stale: Vec<_> = listed.difference(&sources).collect();
        assert!(
            missing.is_empty() && stale.is_empty(),
            "add to KNOWN in crates/hfs/src/env_names.rs: {missing:?}\n\
             remove from it, as no source names them: {stale:?}"
        );
    }

    #[test]
    fn the_list_is_in_byte_order_without_repeats() {
        for pair in KNOWN.windows(2) {
            assert!(pair[0] < pair[1], "{} then {}", pair[0], pair[1]);
        }
    }

    #[test]
    fn a_misspelt_variable_is_named_with_the_one_it_resembles() {
        assert_eq!(
            unknown_variable_warnings(["HFS_AUTH_TENANT_CLAIMS"]),
            [
                "HFS_AUTH_TENANT_CLAIMS is set but no Helios component reads it; \
              did you mean HFS_AUTH_TENANT_CLAIM?"
            ]
        );
        assert_eq!(
            unknown_variable_warnings(["HFS_AUTH_ENABLE"]),
            ["HFS_AUTH_ENABLE is set but no Helios component reads it; \
              did you mean HFS_AUTH_ENABLED?"]
        );
    }

    #[test]
    fn a_variable_like_none_is_named_alone() {
        assert_eq!(
            unknown_variable_warnings(["HFS_NOTHING_LIKE_IT_AT_ALL"]),
            ["HFS_NOTHING_LIKE_IT_AT_ALL is set but no Helios component reads it"]
        );
    }

    #[test]
    fn known_harness_and_other_variables_are_not_warned_about() {
        assert!(
            unknown_variable_warnings([
                "HFS_AUTH_ENABLED",
                "HFS_SERVER_PORT",
                "HFS_TEST_MONGODB_URL",
                "HFS_E2E_BASE_URL",
                "PATH",
                "hfs_auth_enabled",
            ])
            .is_empty()
        );
    }

    #[test]
    fn each_unknown_variable_is_warned_about_once() {
        assert_eq!(
            unknown_variable_warnings(["HFS_NOTHING_LIKE_IT_AT_ALL", "HFS_NOTHING_LIKE_IT_AT_ALL"])
                .len(),
            1
        );
    }

    #[test]
    fn edit_distance_counts_insertions_deletions_and_substitutions() {
        assert_eq!(edit_distance("HFS_A", "HFS_A"), 0);
        assert_eq!(edit_distance("HFS_AB", "HFS_A"), 1);
        assert_eq!(edit_distance("HFS_A", "HFS_B"), 1);
        assert_eq!(edit_distance("", "HFS"), 3);
    }
}
