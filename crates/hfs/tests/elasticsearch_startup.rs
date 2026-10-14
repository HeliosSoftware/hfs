//! `hfs` runs the Elasticsearch backend's startup initialization (index
//! template, nested-object limit, mapping reconcile) once, before it serves,
//! in every `*-elasticsearch` mode, and starts even when that fails (#1506).
//!
//! The start functions used to build the backend with
//! `ElasticsearchBackend::with_shared_registry`, which skips initialization: an
//! index left by an older build was reconciled only on its first write, with a
//! "reindex needed" warning per index instead of one per pass, and the
//! nested-object limit of existing indices was never raised.
//!
//! What is exercised here, and how:
//!
//! - **`sqlite-elasticsearch`, end to end.** The real `hfs` binary is started
//!   on an Elasticsearch 8.15 cluster holding two version-1 indices with
//!   documents in them. Once `/metadata` answers, and before any resource
//!   request, the cluster must show all three steps done and the log exactly
//!   one reindex warning. A second test starts it with Elasticsearch down and
//!   requires it to serve anyway.
//! - **`postgres-`, `mongodb-` and `s3-elasticsearch`, by their source only.**
//!   Each needs its own primary (PostgreSQL, a MongoDB replica set, S3) next to
//!   Elasticsearch. The guard below fails if any of the four start functions
//!   builds its backend without `start_with_shared_registry`, or if `hfs` builds
//!   one any other way. It reads the source: it does not prove the call runs.
//!
//! The first end-to-end test needs Docker, unless `HFS_TEST_ES_URL` names a
//! cluster that is already up with security disabled. Run with:
//! `cargo test -p helios-hfs --features elasticsearch --test elasticsearch_startup`

/// `src/main.rs` up to its test module (whose code builds backends any way it
/// likes), with Windows line endings normalized.
fn production_source() -> String {
    const MAIN_RS: &str = include_str!("../src/main.rs");
    const TEST_MODULE: &str = "\n#[cfg(test)]\nmod tests {";
    let source = MAIN_RS.replace("\r\n", "\n");
    assert_eq!(
        source.matches(TEST_MODULE).count(),
        1,
        "src/main.rs must have exactly one `{TEST_MODULE}`; update this guard if it moved"
    );
    source
        .split(TEST_MODULE)
        .next()
        .unwrap_or_default()
        .to_string()
}

/// The text of every definition of `fn {name}(` in `source`, from the
/// signature to the first line that is a lone `}`: rustfmt closes a top-level
/// item there, and nothing inside one.
fn definitions<'a>(source: &'a str, name: &str) -> Vec<&'a str> {
    let signature = format!("fn {name}(");
    source
        .match_indices(&signature)
        .map(|(start, _)| {
            let rest = &source[start..];
            &rest[..rest.find("\n}\n").map_or(rest.len(), |end| end + 2)]
        })
        .collect()
}

/// Every `*-elasticsearch` mode builds its search backend with the constructor
/// that runs startup initialization, and `hfs` builds none any other way:
/// `with_shared_registry` and `new` both skip it (#1506).
#[test]
fn every_elasticsearch_mode_initializes_its_search_backend_at_startup() {
    const INITIALIZING: &str = "ElasticsearchBackend::start_with_shared_registry(";
    let source = production_source();

    let mut skipping = Vec::new();
    for mode in ["sqlite", "postgres", "mongodb", "s3"] {
        let name = format!("start_{mode}_elasticsearch");
        let definitions = definitions(&source, &name);
        assert!(
            !definitions.is_empty(),
            "`fn {name}` is gone from src/main.rs; update this guard"
        );
        let initializing = definitions
            .iter()
            .filter(|definition| definition.contains(INITIALIZING))
            .count();
        if initializing != 1 {
            skipping.push(name);
        }
    }
    assert!(
        skipping.is_empty(),
        "{skipping:?} must build the Elasticsearch backend with `{INITIALIZING}…)`, which \
         writes the index template, raises the nested-object limit and reconciles the \
         mappings of existing indices before the server starts (#1506)"
    );

    assert_eq!(
        source.matches("with_shared_registry(").count(),
        source.matches("start_with_shared_registry(").count(),
        "`ElasticsearchBackend::with_shared_registry` skips startup initialization; \
         use `start_with_shared_registry` (#1506)"
    );
    assert!(
        !source.contains("ElasticsearchBackend::new("),
        "`ElasticsearchBackend::new` skips startup initialization and the primary's \
         registry; use `start_with_shared_registry` (#1506)"
    );
}

#[cfg(all(feature = "sqlite", feature = "elasticsearch"))]
mod sqlite_elasticsearch {
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use helios_persistence::backends::elasticsearch::{
        ElasticsearchBackend, ElasticsearchConfig, SCHEMA_VERSION, SCHEMA_VERSION_META_KEY,
    };
    use helios_persistence::core::Backend;
    use serde_json::{Value, json};
    use testcontainers::core::IntoContainerPort;
    use testcontainers::runners::AsyncRunner;
    use testcontainers::{ContainerAsync, GenericImage, ImageExt};

    /// The Elasticsearch the Inferno and smoke workflows deploy.
    const ES_IMAGE: (&str, &str) = ("elasticsearch", "8.15.0");
    const ES_STARTUP_TIMEOUT: Duration = Duration::from_secs(300);
    const HFS_STARTUP_TIMEOUT: Duration = Duration::from_secs(120);

    /// The warning the startup reconcile logs, once per pass, about indices
    /// whose documents predate `search_params.date.end` (#1391).
    const REINDEX_WARNING: &str = "were upgraded to a mapping with `search_params.date.end`";
    /// What `start_with_shared_registry` logs when initialization could not run.
    const INITIALIZATION_FAILED: &str = "Elasticsearch startup initialization failed";

    fn http() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("http client")
    }

    fn unique() -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        format!("{}{nanos}", std::process::id())
    }

    /// An index prefix of this run's own. Not `hfs_…`: a template another
    /// `hfs` left on a shared cluster (`hfs_template`, for `hfs_*`) would add
    /// its mapping to the version-1 indices built here.
    fn new_prefix() -> String {
        format!("esstartup{}", unique())
    }

    /// A free port above 18000 (every OS's ephemeral range is), let go so `hfs`
    /// can take it, or so nothing answers on it.
    fn free_port() -> u16 {
        loop {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
            let port = listener.local_addr().expect("address").port();
            if port > 18000 {
                return port;
            }
        }
    }

    struct Cluster {
        url: String,
        _container: Option<ContainerAsync<GenericImage>>,
    }

    async fn cluster() -> Cluster {
        if let Ok(url) = std::env::var("HFS_TEST_ES_URL") {
            return Cluster {
                url,
                _container: None,
            };
        }
        let (name, tag) = ES_IMAGE;
        let container = GenericImage::new(name, tag)
            .with_exposed_port(9200.tcp())
            .with_env_var("discovery.type", "single-node")
            .with_env_var("xpack.security.enabled", "false")
            .with_env_var("ingest.geoip.downloader.enabled", "false")
            .with_env_var("cluster.routing.allocation.disk.threshold_enabled", "false")
            .with_env_var("ES_JAVA_OPTS", "-Xms512m -Xmx512m")
            .start()
            .await
            .expect("start the Elasticsearch container");
        let host = container.get_host().await.expect("container host");
        let port = container
            .get_host_port_ipv4(9200)
            .await
            .expect("container port");
        let url = format!("http://{host}:{port}");

        // 8.x logs no line a generic image can wait for; ask the cluster.
        let deadline = Instant::now() + ES_STARTUP_TIMEOUT;
        loop {
            let health = http()
                .get(format!(
                    "{url}/_cluster/health?wait_for_status=yellow&timeout=5s"
                ))
                .send()
                .await;
            if matches!(&health, Ok(response) if response.status().is_success()) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Elasticsearch at {url} did not come up: {health:?}"
            );
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Cluster {
            url,
            _container: Some(container),
        }
    }

    /// The current index mapping as a version-1 build laid it out: without
    /// `search_params.date.end` (#1391), marked version 1. Read from the
    /// template a backend writes for a scratch prefix, so it follows the
    /// mapping as the mapping changes.
    async fn version_1_mapping(es: &str) -> Value {
        let scratch = new_prefix();
        let backend = ElasticsearchBackend::new(ElasticsearchConfig {
            nodes: vec![es.to_string()],
            index_prefix: scratch.clone(),
            ..Default::default()
        })
        .expect("scratch backend");
        backend
            .initialize()
            .await
            .expect("write the scratch template");
        let template = format!("{scratch}_template");
        let body: Value = http()
            .get(format!("{es}/_template/{template}"))
            .send()
            .await
            .expect("read the scratch template")
            .json()
            .await
            .expect("template json");
        let _ = http()
            .delete(format!("{es}/_template/{template}"))
            .send()
            .await;

        let mut mapping = body[&template]["mappings"].clone();
        let removed = mapping
            .pointer_mut("/properties/search_params/properties/date/properties")
            .and_then(Value::as_object_mut)
            .and_then(|date| date.remove("end"));
        assert!(removed.is_some(), "no `date.end` to remove: {mapping}");
        mapping["_meta"] = json!({ SCHEMA_VERSION_META_KEY: 1 });
        mapping
    }

    /// A document as a build before #1391 indexed it: its date has no `end`.
    fn document_indexed_before_1391(resource_type: &str, id: &str, date: (&str, &str)) -> Value {
        json!({
            "resource_type": resource_type,
            "resource_id": id,
            "tenant_id": "default",
            "version_id": "1",
            "last_updated": "2024-01-01T00:00:00Z",
            "is_deleted": false,
            "content": { "resourceType": resource_type, "id": id },
            "search_params": { "date": [
                { "name": date.0, "value": date.1, "precision": "day" }
            ] }
        })
    }

    async fn mapping_of(es: &str, index: &str) -> Value {
        let body: Value = http()
            .get(format!("{es}/{index}/_mapping"))
            .send()
            .await
            .expect("read mapping")
            .json()
            .await
            .expect("mapping json");
        body[index]["mappings"].clone()
    }

    /// `index.mapping.nested_objects.limit` of `index`: its own value, else
    /// the cluster default.
    async fn nested_objects_limit(es: &str, index: &str) -> u64 {
        let body: Value = http()
            .get(format!(
                "{es}/{index}/_settings/index.mapping.nested_objects.limit\
                 ?include_defaults=true&flat_settings=true"
            ))
            .send()
            .await
            .expect("read settings")
            .json()
            .await
            .expect("settings json");
        let entry = &body[index];
        entry["settings"]["index.mapping.nested_objects.limit"]
            .as_str()
            .or_else(|| entry["defaults"]["index.mapping.nested_objects.limit"].as_str())
            .and_then(|limit| limit.parse().ok())
            .unwrap_or_else(|| panic!("no nested-object limit for {index}: {body}"))
    }

    /// Creates `index` as a version-1 build left it — `mapping`, the default
    /// nested-object limit — holding `document`, and checks that it did.
    async fn create_version_1_index(es: &str, index: &str, mapping: &Value, document: Value) {
        let created = http()
            .put(format!("{es}/{index}"))
            .json(&json!({
                "settings": {
                    "number_of_replicas": 0,
                    "analysis": { "normalizer": { "lowercase_normalizer": {
                        "type": "custom", "filter": ["lowercase"]
                    } } }
                },
                "mappings": mapping
            }))
            .send()
            .await
            .expect("create index");
        let status = created.status();
        assert!(
            status.is_success(),
            "create {index} ({status}): {}",
            created.text().await.unwrap_or_default()
        );
        let id = format!(
            "{}_{}",
            document["resource_type"].as_str().unwrap_or_default(),
            document["resource_id"].as_str().unwrap_or_default()
        );
        let indexed = http()
            .put(format!("{es}/{index}/_doc/{id}?refresh=true"))
            .json(&document)
            .send()
            .await
            .expect("index document");
        let status = indexed.status();
        assert!(
            status.is_success(),
            "index into {index} ({status}): {}",
            indexed.text().await.unwrap_or_default()
        );

        // Positive control: what `hfs` will find.
        let mapping = mapping_of(es, index).await;
        assert_eq!(
            mapping["_meta"][SCHEMA_VERSION_META_KEY],
            json!(1),
            "{mapping}"
        );
        assert!(
            mapping
                .pointer("/properties/search_params/properties/date/properties/end")
                .is_none(),
            "{mapping}"
        );
        assert_eq!(nested_objects_limit(es, index).await, 10_000);
    }

    /// A running `hfs`, killed on drop. Its working directory (database, log)
    /// is removed then, or kept and named when the test failed.
    struct Hfs {
        child: Child,
        port: u16,
        work: PathBuf,
    }

    impl Hfs {
        /// Starts `hfs` in `sqlite-elasticsearch` mode on the cluster at
        /// `es`, with its indices under `prefix`.
        fn start(es: &str, prefix: &str, extra_env: &[(&str, &str)]) -> Hfs {
            let work = Path::new(env!("CARGO_TARGET_TMPDIR"))
                .join(format!("elasticsearch_startup-{}", unique()));
            std::fs::create_dir_all(&work).expect("working directory");
            let port = free_port();
            let log = std::fs::File::create(work.join("hfs.log")).expect("log file");

            let mut command = Command::new(env!("CARGO_BIN_EXE_hfs"));
            // Nothing in the caller's environment decides how this `hfs` runs.
            for (key, _) in std::env::vars_os() {
                if let Some(key) = key.to_str()
                    && (key.starts_with("HFS_") || key.starts_with("OTEL_") || key == "RUST_LOG")
                {
                    command.env_remove(key);
                }
            }
            command
                .current_dir(&work)
                .env("HFS_STORAGE_BACKEND", "sqlite-elasticsearch")
                .env("HFS_ELASTICSEARCH_NODES", es)
                .env("HFS_ELASTICSEARCH_INDEX_PREFIX", prefix)
                .env("HFS_COMPOSITE_SYNC_MODE", "synchronous")
                .env("HFS_DATABASE_URL", work.join("fhir.db"))
                .env(
                    "HFS_DATA_DIR",
                    concat!(env!("CARGO_MANIFEST_DIR"), "/../../data"),
                )
                .env("HFS_SERVER_HOST", "127.0.0.1")
                .env("HFS_SERVER_PORT", port.to_string())
                .env("HFS_BASE_URL", format!("http://127.0.0.1:{port}"))
                .env("HFS_UI_ENABLED", "false")
                // ~1,400 writes through the composite, none of them to the
                // indices under test.
                .env("HFS_SEED_CONFORMANCE", "false")
                .env("HFS_LOG_LEVEL", "info")
                .envs(extra_env.iter().copied())
                .stdin(Stdio::null())
                .stdout(log.try_clone().expect("log file"))
                .stderr(log);
            let child = command.spawn().expect("start hfs");
            Hfs { child, port, work }
        }

        fn url(&self, path: &str) -> String {
            format!("http://127.0.0.1:{}{path}", self.port)
        }

        /// Waits until `/metadata` answers `200`.
        async fn wait_until_serving(&mut self) {
            let deadline = Instant::now() + HFS_STARTUP_TIMEOUT;
            loop {
                if let Ok(response) = http().get(self.url("/metadata")).send().await
                    && response.status().is_success()
                {
                    return;
                }
                if let Some(status) = self.child.try_wait().expect("poll hfs") {
                    panic!("hfs exited ({status}) before serving:\n{}", self.log());
                }
                assert!(
                    Instant::now() < deadline,
                    "hfs did not answer /metadata within {HFS_STARTUP_TIMEOUT:?}:\n{}",
                    self.log()
                );
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }

        fn log(&self) -> String {
            std::fs::read_to_string(self.work.join("hfs.log")).unwrap_or_default()
        }

        fn log_lines_containing(&self, needle: &str) -> Vec<String> {
            self.log()
                .lines()
                .filter(|line| line.contains(needle))
                .map(str::to_string)
                .collect()
        }
    }

    impl Drop for Hfs {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            if std::thread::panicking() {
                eprintln!("hfs working directory kept: {}", self.work.display());
            } else {
                let _ = std::fs::remove_dir_all(&self.work);
            }
        }
    }

    /// The `hfs` half of #1506. An Elasticsearch whose indices an older build
    /// left — mapping version 1, the default nested-object limit, documents
    /// indexed before `search_params.date.end` — is brought up to date before
    /// `hfs` serves: by the time `/metadata` answers, and before any resource
    /// request, the template is written, both indices carry the current
    /// mapping and the configured limit, and one warning covers them both.
    /// The first write afterwards does not warn again.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sqlite_elasticsearch_initializes_the_cluster_before_serving() {
        let cluster = cluster().await;
        let es = cluster.url.as_str();
        let prefix = new_prefix();
        let names = ElasticsearchBackend::new(ElasticsearchConfig {
            nodes: vec![es.to_string()],
            index_prefix: prefix.clone(),
            ..Default::default()
        })
        .expect("index names");
        let patient = names.index_name("default", "Patient");
        let observation = names.index_name("default", "Observation");

        let mapping = version_1_mapping(es).await;
        create_version_1_index(
            es,
            &patient,
            &mapping,
            document_indexed_before_1391("Patient", "p1", ("birthdate", "1980-05-17")),
        )
        .await;
        create_version_1_index(
            es,
            &observation,
            &mapping,
            document_indexed_before_1391("Observation", "o1", ("date", "2020-03-01")),
        )
        .await;

        let mut hfs = Hfs::start(es, &prefix, &[]);
        hfs.wait_until_serving().await;

        // Nothing has been asked of `hfs` but `/metadata`.
        let template = http()
            .get(format!("{es}/_template/{prefix}_template"))
            .send()
            .await
            .expect("read template");
        assert!(
            template.status().is_success(),
            "the index template was not written at startup:\n{}",
            hfs.log()
        );
        for index in [&patient, &observation] {
            let mapping = mapping_of(es, index).await;
            assert_eq!(
                mapping["_meta"][SCHEMA_VERSION_META_KEY],
                json!(SCHEMA_VERSION),
                "{index} was not reconciled at startup:\n{}",
                hfs.log()
            );
            assert_eq!(
                mapping.pointer("/properties/search_params/properties/date/properties/end/type"),
                Some(&json!("date")),
                "{index}: {mapping}"
            );
            assert_eq!(
                nested_objects_limit(es, index).await,
                50_000,
                "{index}'s nested-object limit was not raised at startup:\n{}",
                hfs.log()
            );
        }
        let warnings = hfs.log_lines_containing(REINDEX_WARNING);
        assert_eq!(warnings.len(), 1, "one per startup pass:\n{}", hfs.log());
        assert!(warnings[0].contains("indices=2"), "{}", warnings[0]);
        assert!(
            hfs.log_lines_containing(INITIALIZATION_FAILED).is_empty(),
            "{}",
            hfs.log()
        );

        let created = http()
            .post(hfs.url("/Patient"))
            .header("Content-Type", "application/fhir+json")
            .body(
                json!({ "resourceType": "Patient", "name": [{ "family": "Startup" }] }).to_string(),
            )
            .send()
            .await
            .expect("create a Patient");
        assert_eq!(created.status().as_u16(), 201, "{}", hfs.log());
        assert_eq!(
            hfs.log_lines_containing(REINDEX_WARNING).len(),
            1,
            "the first write must not warn about an index the startup pass reconciled:\n{}",
            hfs.log()
        );

        drop(hfs);
        // Matters only on a shared cluster (`HFS_TEST_ES_URL`).
        for path in [patient, observation, format!("_template/{prefix}_template")] {
            let _ = http().delete(format!("{es}/{path}")).send().await;
        }
    }

    /// Elasticsearch down at boot: `hfs` logs it once and serves anyway.
    /// Nothing listens on the port, so the connection is refused at once; the
    /// short request timeout bounds the start where it is dropped instead.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sqlite_elasticsearch_serves_with_elasticsearch_down() {
        let es = format!("http://127.0.0.1:{}", free_port());
        let mut hfs = Hfs::start(
            &es,
            &new_prefix(),
            &[("HFS_ELASTICSEARCH_REQUEST_TIMEOUT_MS", "2000")],
        );
        hfs.wait_until_serving().await;

        let failures = hfs.log_lines_containing(INITIALIZATION_FAILED);
        assert_eq!(failures.len(), 1, "{}", hfs.log());
        assert!(failures[0].contains("did not answer"), "{}", failures[0]);
        // Nothing past the reachability check was tried.
        for step in [
            "could not create the Elasticsearch index template",
            "could not raise the Elasticsearch nested-object limit",
            "could not reconcile the Elasticsearch index mapping",
            REINDEX_WARNING,
        ] {
            assert!(
                hfs.log_lines_containing(step).is_empty(),
                "{step}:\n{}",
                hfs.log()
            );
        }
    }
}
