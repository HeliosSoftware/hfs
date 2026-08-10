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
///
/// v7 adds the Bulk Data Submit collections and their indexes.
pub const SCHEMA_VERSION: i32 = 7;

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
    ensure_tenants_indexes(database).await?;
    ensure_bulk_submit_indexes(database).await?;
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
        ensure_tenants_indexes(database).await?;
        ensure_bulk_submit_indexes(database).await?;
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

/// Index for the tenant registry (schema v6). One document per registered
/// tenant, keyed by a unique `id`, so a concurrent double-register surfaces as
/// a duplicate-key error instead of two rows. See the SQLite `tenants` table
/// (schema v14) for the canonical shape: `id`, optional `display_name`, and an
/// RFC 3339 `created_at`.
async fn ensure_tenants_indexes(database: &Database) -> StorageResult<()> {
    let tenants = database.collection::<Document>("tenants");

    create_index(&tenants, doc! { "id": 1_i32 }, "idx_tenants_id", true).await?;

    Ok(())
}

/// Indexes for the Bulk Data Submit collections (schema v7).
///
/// MongoDB hosts the `$bulk-submit` job store natively (see
/// [`super::bulk_submit`]), so these indexes back three distinct access
/// patterns, not just lookups:
///
/// - the per-submission and per-manifest reads every provider method issues,
/// - the **cross-tenant** worker scans — claiming the next pending manifest,
///   resolving a poll token, and the TTL sweep over `updated_at` — which
///   deliberately carry no `tenant_id` and so need their own indexes,
/// - the uniqueness that makes an entry result idempotent: `(manifest,
///   file_url, line_number)`. The `file_url` is part of the key because line
///   numbers restart in every manifest output file (#457).
async fn ensure_bulk_submit_indexes(database: &Database) -> StorageResult<()> {
    use super::bulk_submit::{
        CHANGES_COLLECTION, ENTRY_RESULTS_COLLECTION, MANIFESTS_COLLECTION, SUBMISSIONS_COLLECTION,
        SUBMIT_FILES_COLLECTION,
    };

    let submission_key = doc! {
        "tenant_id": 1_i32, "submitter": 1_i32, "submission_id": 1_i32,
    };

    let submissions = database.collection::<Document>(SUBMISSIONS_COLLECTION);
    create_index(
        &submissions,
        submission_key.clone(),
        "idx_bulk_submissions_id",
        true,
    )
    .await?;
    create_index(
        &submissions,
        doc! { "tenant_id": 1_i32, "status": 1_i32 },
        "idx_bulk_submissions_status",
        false,
    )
    .await?;
    // Poll-token resolution and the TTL sweep run without a tenant in hand.
    create_index_with(
        &submissions,
        doc! { "poll_token": 1_i32 },
        IndexOptions::builder()
            .name(Some("idx_bulk_submissions_poll_token".to_string()))
            .unique(Some(true))
            .sparse(Some(true))
            .build(),
    )
    .await?;
    create_index(
        &submissions,
        doc! { "updated_at": 1_i32 },
        "idx_bulk_submissions_updated_at",
        false,
    )
    .await?;

    let manifests = database.collection::<Document>(MANIFESTS_COLLECTION);
    let mut manifest_key = submission_key.clone();
    manifest_key.insert("manifest_id", 1_i32);
    create_index(&manifests, manifest_key, "idx_bulk_manifests_id", true).await?;
    // The claim scan is cross-tenant and ordered by `added_at`.
    create_index(
        &manifests,
        doc! { "status": 1_i32, "lease_expiry": 1_i32, "added_at": 1_i32 },
        "idx_bulk_manifests_claim",
        false,
    )
    .await?;

    let entry_results = database.collection::<Document>(ENTRY_RESULTS_COLLECTION);
    let mut entry_key = submission_key.clone();
    entry_key.insert("manifest_id", 1_i32);
    entry_key.insert("file_url", 1_i32);
    entry_key.insert("line_number", 1_i32);
    create_index(
        &entry_results,
        entry_key,
        "idx_bulk_entry_results_line",
        true,
    )
    .await?;
    let mut outcome_key = submission_key.clone();
    outcome_key.insert("manifest_id", 1_i32);
    outcome_key.insert("outcome", 1_i32);
    create_index(
        &entry_results,
        outcome_key,
        "idx_bulk_entry_results_outcome",
        false,
    )
    .await?;

    let changes = database.collection::<Document>(CHANGES_COLLECTION);
    let mut change_key = submission_key.clone();
    change_key.insert("change_id", 1_i32);
    create_index(&changes, change_key, "idx_bulk_changes_id", true).await?;

    let files = database.collection::<Document>(SUBMIT_FILES_COLLECTION);
    let mut file_key = submission_key;
    file_key.insert("file_type", 1_i32);
    file_key.insert("resource_type", 1_i32);
    file_key.insert("part_index", 1_i32);
    file_key.insert("fencing_token", 1_i32);
    create_index(&files, file_key, "idx_bulk_submit_files_part", true).await?;

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

    create_index_with(collection, keys, options).await
}

async fn create_index_with(
    collection: &Collection<Document>,
    keys: Document,
    options: IndexOptions,
) -> StorageResult<()> {
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
