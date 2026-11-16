//! A create attempt of an id it was told the backend minted, in a transaction,
//! sends no existence check: the `resources` identity index is what still
//! answers a duplicate as `AlreadyExists`. A minted id never collides through
//! the public API, so these tests call [`MongoBackend::create_attempt`]
//! directly with an id that is already stored, and pin that the duplicate-key
//! mapping answers `AlreadyExists` and that the attempt writes nothing.
//!
//! They need a replica set (a standalone server runs the attempt without a
//! transaction, which keeps the check). The server is `HFS_TEST_MONGODB_URL`
//! when that is a replica set, otherwise a `mongo:7.0` container started as a
//! single-node replica set; without either the tests skip.

use super::*;
use crate::backends::mongodb::MongoBackendConfig;
use crate::core::Backend;
use crate::tenant::{TenantId, TenantPermissions};

mod replica_set {
    use mongodb::bson::doc;
    use testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner};
    use testcontainers_modules::mongo::Mongo;
    use tokio::sync::OnceCell;

    struct Shared {
        connection_string: String,
        /// Never dropped; the `container_cleanup` exit hook removes it.
        _container: Option<ContainerAsync<Mongo>>,
    }

    static SHARED: OnceCell<Result<Shared, String>> = OnceCell::const_new();

    /// `Err` carries the reason no replica set is available.
    pub(super) async fn connection_string() -> Result<String, String> {
        SHARED
            .get_or_init(|| async {
                match std::env::var("HFS_TEST_MONGODB_URL") {
                    Ok(url) => {
                        let client = mongodb::Client::with_uri_str(&url)
                            .await
                            .map_err(|e| format!("HFS_TEST_MONGODB_URL unusable ({e})"))?;
                        let hello = client
                            .database("admin")
                            .run_command(doc! { "hello": 1_i32 })
                            .await
                            .map_err(|e| format!("HFS_TEST_MONGODB_URL unreachable ({e})"))?;
                        if hello.get_str("setName").is_err() {
                            return Err("HFS_TEST_MONGODB_URL is not a replica set".to_string());
                        }
                        Ok(Shared {
                            connection_string: url,
                            _container: None,
                        })
                    }
                    Err(_) => start().await,
                }
            })
            .await
            .as_ref()
            .map(|shared| shared.connection_string.clone())
            .map_err(Clone::clone)
    }

    async fn start() -> Result<Shared, String> {
        let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
        let container = crate::container_cleanup::with_cleanup_label(
            Mongo::default()
                .with_tag("7.0")
                .with_label("github.run_id", &run_id)
                .with_ulimit("nofile", 65_536, Some(65_536))
                .with_cmd([
                    "mongod",
                    "--bind_ip_all",
                    "--wiredTigerCacheSizeGB",
                    "0.25",
                    "--replSet",
                    "rs0",
                ]),
        )
        .start()
        .await
        .map_err(|e| format!("the Mongo container did not start ({e})"))?;
        let host = container
            .get_host()
            .await
            .map_err(|e| format!("Mongo container host unavailable ({e})"))?;
        let port = container
            .get_host_port_ipv4(27017)
            .await
            .map_err(|e| format!("Mongo container port unavailable ({e})"))?;
        // The member advertises its address inside the container, so the
        // host connects to it directly.
        let connection_string = format!("mongodb://{host}:{port}/?directConnection=true");
        let client = mongodb::Client::with_uri_str(&connection_string)
            .await
            .map_err(|e| format!("Mongo container unusable ({e})"))?;
        let admin = client.database("admin");
        admin
            .run_command(doc! {
                "replSetInitiate": {
                    "_id": "rs0",
                    "members": [{ "_id": 0_i32, "host": "localhost:27017" }],
                }
            })
            .await
            .map_err(|e| format!("replSetInitiate failed ({e})"))?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            let primary = admin
                .run_command(doc! { "hello": 1_i32 })
                .await
                .ok()
                .and_then(|hello| hello.get_bool("isWritablePrimary").ok())
                .unwrap_or(false);
            if primary {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err("the replica set elected no primary within 60 s".to_string());
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        Ok(Shared {
            connection_string,
            _container: Some(container),
        })
    }
}

/// Reports a skip, or fails when `HFS_TEST_REQUIRE_MONGODB` is set.
fn skip(label: &str, reason: &str) {
    if std::env::var("HFS_TEST_REQUIRE_MONGODB").is_ok_and(|value| !value.is_empty()) {
        panic!("{label}: {reason}; HFS_TEST_REQUIRE_MONGODB forbids skipping");
    }
    eprintln!("Skipping {label}: {reason}");
}

struct Fixture {
    backend: MongoBackend,
    db: mongodb::Database,
    tenant: TenantContext,
}

impl Fixture {
    async fn new(label: &str) -> Option<Self> {
        let connection_string = match replica_set::connection_string().await {
            Ok(url) => url,
            Err(reason) => {
                skip(label, &reason);
                return None;
            }
        };
        let database_name = format!("hfs_create_cmd_{label}_{}", uuid::Uuid::new_v4().simple());
        let backend = MongoBackend::new(MongoBackendConfig {
            connection_string: connection_string.clone(),
            database_name: database_name.clone(),
            data_dir: Some(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")),
            ..Default::default()
        })
        .expect("backend");
        backend.initialize().await.expect("schema");
        let db = mongodb::Client::with_uri_str(&connection_string)
            .await
            .unwrap()
            .database(&database_name);
        let tenant = TenantContext::new(TenantId::new(label), TenantPermissions::full_access());
        Some(Self {
            backend,
            db,
            tenant,
        })
    }

    async fn count(&self, collection: &str, filter: Document) -> u64 {
        self.db
            .collection::<Document>(collection)
            .count_documents(filter)
            .await
            .unwrap()
    }

    /// `resources`, `resource_history` and `search_index` rows under `id`.
    async fn rows(&self, id: &str) -> (u64, u64, u64) {
        (
            self.count("resources", doc! { "id": id }).await,
            self.count("resource_history", doc! { "id": id }).await,
            self.count("search_index", doc! { "resource_id": id }).await,
        )
    }

    async fn drop(self) {
        let _ = self.db.drop().await;
    }
}

fn observation(id: &str, status: &str) -> Value {
    serde_json::json!({
        "resourceType": "Observation",
        "id": id,
        "status": status,
        "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
    })
}

/// Runs one create attempt of `id` over a stored resource and returns its
/// result, after aborting its transaction as [`ResourceStorage::create`] does.
async fn attempt_over_stored(fixture: &Fixture, id: &str, id_assigned: bool) -> StorageResult<()> {
    let mut session = None;
    let attempt = fixture
        .backend
        .create_attempt(
            &fixture.tenant,
            "Observation",
            id,
            id_assigned,
            &observation(id, "amended"),
            FhirVersion::default(),
            &mut session,
        )
        .await;
    assert!(
        session.is_some(),
        "the attempt ran in a transaction, so it took the path under test"
    );
    abort_failed_write(&mut session, attempt, aborts_failed_write)
        .await
        .map(|_| ())
}

/// An attempt told its id was minted skips the existence check; the identity
/// index still refuses the stored id, and the answer is `AlreadyExists` with
/// nothing written.
#[tokio::test]
async fn a_minted_id_already_stored_is_already_exists_and_writes_nothing() {
    let Some(fixture) = Fixture::new("minted_dup").await else {
        return;
    };
    fixture
        .backend
        .create(
            &fixture.tenant,
            "Observation",
            observation("obs-dup", "final"),
            FhirVersion::default(),
        )
        .await
        .expect("the first create commits");
    let before = fixture.rows("obs-dup").await;

    let result = attempt_over_stored(&fixture, "obs-dup", true).await;

    assert!(
        matches!(
            result,
            Err(StorageError::Resource(ResourceError::AlreadyExists { ref id, .. }))
                if id == "obs-dup"
        ),
        "{result:?}"
    );
    assert_eq!(fixture.rows("obs-dup").await, before);
    let stored = fixture
        .backend
        .read(&fixture.tenant, "Observation", "obs-dup")
        .await
        .unwrap()
        .expect("the first create's resource");
    assert_eq!(stored.content()["status"], "final");
    assert_eq!(stored.version_id(), "1");
    fixture.drop().await;
}

/// The same attempt for a client-supplied id, which keeps the existence
/// check: the same answer, nothing written.
#[tokio::test]
async fn a_client_id_already_stored_is_already_exists_and_writes_nothing() {
    let Some(fixture) = Fixture::new("client_dup").await else {
        return;
    };
    fixture
        .backend
        .create(
            &fixture.tenant,
            "Observation",
            observation("obs-dup", "final"),
            FhirVersion::default(),
        )
        .await
        .expect("the first create commits");
    let before = fixture.rows("obs-dup").await;

    let result = attempt_over_stored(&fixture, "obs-dup", false).await;

    assert!(
        matches!(
            result,
            Err(StorageError::Resource(ResourceError::AlreadyExists { .. }))
        ),
        "{result:?}"
    );
    assert_eq!(fixture.rows("obs-dup").await, before);
    fixture.drop().await;
}
