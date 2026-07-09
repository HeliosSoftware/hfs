//! MongoDB schema/bootstrap helpers.

use mongodb::{
    Client, Collection, Database, IndexModel,
    bson::{Document, doc},
    options::{ClientOptions, IndexOptions},
};
use tokio::runtime::RuntimeFlavor;

use crate::error::{BackendError, StorageError, StorageResult};

use super::backend::MongoBackendConfig;

/// Current MongoDB schema version.
pub const SCHEMA_VERSION: i32 = 5;

/// Initialize MongoDB collections/indexes required by the backend.
///
/// Prefer using [`initialize_schema_async`] from async contexts.
#[allow(dead_code)]
pub fn initialize_schema(config: &MongoBackendConfig) -> StorageResult<()> {
    run_with_runtime(async {
        let client = create_client(config).await?;
        let db = client.database(&config.database_name);
        initialize_schema_async(&db).await
    })
}

/// Run pending MongoDB schema/index migrations.
///
/// Prefer using [`migrate_schema_async`] from async contexts.
#[allow(dead_code)]
pub fn migrate_schema(config: &MongoBackendConfig) -> StorageResult<()> {
    run_with_runtime(async {
        let client = create_client(config).await?;
        let db = client.database(&config.database_name);
        migrate_schema_async(&db).await
    })
}

/// Initialize the MongoDB schema and indexes asynchronously.
pub async fn initialize_schema_async(database: &Database) -> StorageResult<()> {
    ensure_resources_indexes(database).await?;
    ensure_history_indexes(database).await?;
    ensure_search_indexes(database).await?;
    ensure_user_settings_indexes(database).await?;
    set_schema_version(database, SCHEMA_VERSION).await?;
    Ok(())
}

/// Run pending MongoDB schema/index migrations asynchronously.
pub async fn migrate_schema_async(database: &Database) -> StorageResult<()> {
    let current = get_schema_version(database).await?;
    if current < SCHEMA_VERSION {
        ensure_resources_indexes(database).await?;
        ensure_history_indexes(database).await?;
        ensure_search_indexes(database).await?;
        ensure_user_settings_indexes(database).await?;
        set_schema_version(database, SCHEMA_VERSION).await?;
    }
    Ok(())
}

#[allow(dead_code)]
async fn create_client(config: &MongoBackendConfig) -> StorageResult<Client> {
    let mut options = ClientOptions::parse(&config.connection_string)
        .await
        .map_err(|e| {
            StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: "mongodb".to_string(),
                message: e.to_string(),
            })
        })?;

    options.max_pool_size = Some(config.max_connections);
    options.connect_timeout = Some(std::time::Duration::from_millis(config.connect_timeout_ms));
    options.app_name = Some("helios-persistence".to_string());

    Client::with_options(options).map_err(|e| {
        StorageError::Backend(BackendError::Internal {
            backend_name: "mongodb".to_string(),
            message: format!("Failed to create MongoDB client: {}", e),
            source: None,
        })
    })
}

async fn ensure_resources_indexes(database: &Database) -> StorageResult<()> {
    let resources = database.collection::<Document>("resources");

    create_index(
        &resources,
        doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "id": 1_i32 },
        "idx_resources_identity",
        true,
    )
    .await?;

    create_index(
        &resources,
        doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "is_deleted": 1_i32 },
        "idx_resources_type_deleted",
        false,
    )
    .await?;

    create_index(
        &resources,
        doc! { "tenant_id": 1_i32, "last_updated": -1_i32 },
        "idx_resources_updated",
        false,
    )
    .await?;

    Ok(())
}

async fn ensure_history_indexes(database: &Database) -> StorageResult<()> {
    let history = database.collection::<Document>("resource_history");

    create_index(
        &history,
        doc! {
            "tenant_id": 1_i32,
            "resource_type": 1_i32,
            "id": 1_i32,
            "version_id": 1_i32
        },
        "idx_history_identity",
        true,
    )
    .await?;

    create_index(
        &history,
        doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "id": 1_i32, "last_updated": -1_i32 },
        "idx_history_resource_updated",
        false,
    )
    .await?;

    create_index(
        &history,
        doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "last_updated": -1_i32, "id": -1_i32 },
        "idx_history_type_updated",
        false,
    )
    .await?;

    create_index(
        &history,
        doc! { "tenant_id": 1_i32, "last_updated": -1_i32, "resource_type": -1_i32, "id": -1_i32 },
        "idx_history_system_updated",
        false,
    )
    .await?;

    Ok(())
}

async fn ensure_search_indexes(database: &Database) -> StorageResult<()> {
    let search_index = database.collection::<Document>("search_index");

    create_index(
        &search_index,
        doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "param_name": 1_i32, "value_string": 1_i32 },
        "idx_search_string",
        false,
    )
    .await?;

    create_index(
        &search_index,
        doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "param_name": 1_i32, "value_token_system": 1_i32, "value_token_code": 1_i32 },
        "idx_search_token",
        false,
    )
    .await?;

    create_index(
        &search_index,
        doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "param_name": 1_i32, "value_date": 1_i32 },
        "idx_search_date",
        false,
    )
    .await?;

    create_index(
        &search_index,
        doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "param_name": 1_i32, "value_number": 1_i32 },
        "idx_search_number",
        false,
    )
    .await?;

    create_index(
        &search_index,
        doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "param_name": 1_i32, "value_quantity_value": 1_i32, "value_quantity_unit": 1_i32 },
        "idx_search_quantity",
        false,
    )
    .await?;

    create_index(
        &search_index,
        doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "param_name": 1_i32, "value_reference": 1_i32 },
        "idx_search_reference",
        false,
    )
    .await?;

    create_index(
        &search_index,
        doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "param_name": 1_i32, "value_uri": 1_i32 },
        "idx_search_uri",
        false,
    )
    .await?;

    create_index(
        &search_index,
        doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "resource_id": 1_i32, "param_name": 1_i32, "composite_group": 1_i32 },
        "idx_search_composite",
        false,
    )
    .await?;

    create_index(
        &search_index,
        doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "resource_id": 1_i32 },
        "idx_search_resource",
        false,
    )
    .await?;

    create_index(
        &search_index,
        doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "param_name": 1_i32, "value_token_display": 1_i32 },
        "idx_search_token_display",
        false,
    )
    .await?;

    create_index(
        &search_index,
        doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "param_name": 1_i32, "value_identifier_type_system": 1_i32, "value_identifier_type_code": 1_i32 },
        "idx_search_identifier_type",
        false,
    )
    .await?;

    Ok(())
}

/// Index for the per-user UI settings store. One document per user, keyed by a
/// unique `user_key`, so the settings store can rely on the index to serialize
/// concurrent first-writes (a lost insert race surfaces as a duplicate-key
/// error). Kept separate from the FHIR `resources` collection so UI preferences
/// never surface in FHIR machinery (`CapabilityStatement`, search, history,
/// export).
async fn ensure_user_settings_indexes(database: &Database) -> StorageResult<()> {
    let user_settings =
        database.collection::<Document>(super::user_settings::USER_SETTINGS_COLLECTION);

    create_index(
        &user_settings,
        doc! { "user_key": 1_i32 },
        "idx_user_settings_key",
        true,
    )
    .await?;

    Ok(())
}

async fn create_index(
    collection: &Collection<Document>,
    keys: Document,
    name: &str,
    unique: bool,
) -> StorageResult<()> {
    let options = IndexOptions::builder()
        .name(Some(name.to_string()))
        .unique(Some(unique))
        .build();

    let model = IndexModel::builder()
        .keys(keys)
        .options(Some(options))
        .build();
    collection.create_index(model).await?;
    Ok(())
}

async fn get_schema_version(database: &Database) -> StorageResult<i32> {
    let collection = database.collection::<Document>("schema_version");
    let doc = collection
        .find_one(doc! { "_id": "schema_version" })
        .await?;
    let version = doc.and_then(|d| d.get_i32("version").ok()).unwrap_or(0_i32);
    Ok(version)
}

async fn set_schema_version(database: &Database, version: i32) -> StorageResult<()> {
    let collection = database.collection::<Document>("schema_version");
    collection
        .delete_many(doc! { "_id": "schema_version" })
        .await?;
    collection
        .insert_one(doc! {
            "_id": "schema_version",
            "version": version,
        })
        .await?;
    Ok(())
}

#[allow(dead_code)]
fn run_with_runtime<F>(future: F) -> StorageResult<()>
where
    F: std::future::Future<Output = StorageResult<()>>,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        match handle.runtime_flavor() {
            RuntimeFlavor::MultiThread => tokio::task::block_in_place(|| handle.block_on(future)),
            RuntimeFlavor::CurrentThread => Err(StorageError::Backend(BackendError::Internal {
                backend_name: "mongodb".to_string(),
                message: "Cannot run synchronous MongoDB schema initialization inside a current-thread runtime; call Backend::initialize().await instead".to_string(),
                source: None,
            })),
            _ => tokio::task::block_in_place(|| handle.block_on(future)),
        }
    } else {
        let rt = tokio::runtime::Runtime::new().map_err(|e| {
            StorageError::Backend(BackendError::Internal {
                backend_name: "mongodb".to_string(),
                message: format!("Failed to create runtime for schema initialization: {}", e),
                source: None,
            })
        })?;
        rt.block_on(future)
    }
}
