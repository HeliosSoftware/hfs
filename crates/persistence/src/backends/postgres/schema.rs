//! PostgreSQL schema definitions and migrations.

use crate::error::{BackendError, StorageResult};

/// Current schema version.
pub const SCHEMA_VERSION: i32 = 15;

/// Advisory-lock key serializing schema migration across HFS instances sharing
/// one database. Arbitrary but must stay stable across releases.
const MIGRATION_LOCK_KEY: i64 = 0x4846_5300_4d49_4752; // "HFS\0MIGR"

/// Initialize the database schema.
///
/// Serialized across instances with a session-level advisory lock: several HFS
/// processes routinely share one database (see `.github/workflows/cluster-smoke.yml`,
/// which runs two), and `schema_version` is read-then-written without a
/// transaction. While every migration was millisecond-fast the race window was
/// invisible; v15 builds indexes over the whole `search_index` table, which
/// widens it to minutes. Without the lock, a second instance can observe the old
/// version, re-run the migration, and begin serving traffic while the first is
/// still building.
///
/// The lock is session-scoped, so it is released even if the process is killed
/// mid-migration.
pub async fn initialize_schema(client: &deadpool_postgres::Client) -> StorageResult<()> {
    client
        .execute("SELECT pg_advisory_lock($1)", &[&MIGRATION_LOCK_KEY])
        .await
        .map_err(|e| pg_error(format!("Failed to acquire migration lock: {}", e)))?;

    let result = run_migrations(client).await;

    // Release even on failure; the connection may be recycled into the pool.
    if let Err(e) = client
        .execute("SELECT pg_advisory_unlock($1)", &[&MIGRATION_LOCK_KEY])
        .await
    {
        tracing::warn!("Failed to release migration advisory lock: {}", e);
    }

    result
}

async fn run_migrations(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let current_version = get_schema_version(client).await?;

    if current_version == 0 {
        create_schema_v1(client).await?;
        set_schema_version(client, 1).await?;
        migrate_schema(client, 1).await?;
    } else if current_version < SCHEMA_VERSION {
        migrate_schema(client, current_version).await?;
    }

    Ok(())
}

/// Get the current schema version.
async fn get_schema_version(client: &deadpool_postgres::Client) -> StorageResult<i32> {
    client
        .execute(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER NOT NULL
            )",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create schema_version table: {}", e)))?;

    let row = client
        .query_opt("SELECT version FROM schema_version LIMIT 1", &[])
        .await
        .map_err(|e| pg_error(format!("Failed to query schema version: {}", e)))?;

    Ok(row.map(|r| r.get::<_, i32>(0)).unwrap_or(0))
}

/// Set the schema version.
async fn set_schema_version(client: &deadpool_postgres::Client, version: i32) -> StorageResult<()> {
    client
        .execute("DELETE FROM schema_version", &[])
        .await
        .map_err(|e| pg_error(format!("Failed to clear schema_version: {}", e)))?;

    client
        .execute(
            "INSERT INTO schema_version (version) VALUES ($1)",
            &[&version],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to set schema_version: {}", e)))?;

    Ok(())
}

/// Create the initial schema (version 1).
async fn create_schema_v1(client: &deadpool_postgres::Client) -> StorageResult<()> {
    // Main resources table
    client
        .execute(
            "CREATE TABLE IF NOT EXISTS resources (
                tenant_id TEXT NOT NULL,
                resource_type TEXT NOT NULL,
                id TEXT NOT NULL,
                version_id TEXT NOT NULL,
                data JSONB NOT NULL,
                last_updated TIMESTAMPTZ NOT NULL,
                is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
                deleted_at TIMESTAMPTZ,
                PRIMARY KEY (tenant_id, resource_type, id)
            )",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create resources table: {}", e)))?;

    // Resource history table
    client
        .execute(
            "CREATE TABLE IF NOT EXISTS resource_history (
                tenant_id TEXT NOT NULL,
                resource_type TEXT NOT NULL,
                id TEXT NOT NULL,
                version_id TEXT NOT NULL,
                data JSONB NOT NULL,
                last_updated TIMESTAMPTZ NOT NULL,
                is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
                PRIMARY KEY (tenant_id, resource_type, id, version_id)
            )",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create resource_history table: {}", e)))?;

    // Search index table
    client
        .execute(
            "CREATE TABLE IF NOT EXISTS search_index (
                id BIGSERIAL PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                resource_type TEXT NOT NULL,
                resource_id TEXT NOT NULL,
                param_name TEXT NOT NULL,
                param_url TEXT,
                value_string TEXT,
                value_token_system TEXT,
                value_token_code TEXT,
                value_token_display TEXT,
                value_date TIMESTAMPTZ,
                value_date_precision TEXT,
                value_number DOUBLE PRECISION,
                value_quantity_value DOUBLE PRECISION,
                value_quantity_unit TEXT,
                value_quantity_system TEXT,
                value_reference TEXT,
                value_uri TEXT,
                composite_group INTEGER,
                value_identifier_type_system TEXT,
                value_identifier_type_code TEXT,
                value_reference_display TEXT,
                CONSTRAINT fk_search_resource FOREIGN KEY (tenant_id, resource_type, resource_id)
                    REFERENCES resources(tenant_id, resource_type, id) ON DELETE CASCADE
            )",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create search_index table: {}", e)))?;

    // Create indexes
    create_indexes(client).await?;

    // Create FTS tables
    create_fts_tables(client).await?;

    Ok(())
}

/// Create indexes for efficient queries.
async fn create_indexes(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let indexes = [
        // Resources table indexes
        "CREATE INDEX IF NOT EXISTS idx_resources_type ON resources(tenant_id, resource_type)",
        "CREATE INDEX IF NOT EXISTS idx_resources_updated ON resources(tenant_id, last_updated)",
        // History table indexes
        "CREATE INDEX IF NOT EXISTS idx_history_resource ON resource_history(tenant_id, resource_type, id)",
        "CREATE INDEX IF NOT EXISTS idx_history_updated ON resource_history(tenant_id, last_updated)",
        // Search index indexes
        "CREATE INDEX IF NOT EXISTS idx_search_string ON search_index(tenant_id, resource_type, param_name, value_string)",
        "CREATE INDEX IF NOT EXISTS idx_search_token ON search_index(tenant_id, resource_type, param_name, value_token_system, value_token_code)",
        "CREATE INDEX IF NOT EXISTS idx_search_date ON search_index(tenant_id, resource_type, param_name, value_date)",
        "CREATE INDEX IF NOT EXISTS idx_search_number ON search_index(tenant_id, resource_type, param_name, value_number)",
        "CREATE INDEX IF NOT EXISTS idx_search_quantity ON search_index(tenant_id, resource_type, param_name, value_quantity_value, value_quantity_unit)",
        "CREATE INDEX IF NOT EXISTS idx_search_reference ON search_index(tenant_id, resource_type, param_name, value_reference)",
        "CREATE INDEX IF NOT EXISTS idx_search_uri ON search_index(tenant_id, resource_type, param_name, value_uri)",
        "CREATE INDEX IF NOT EXISTS idx_search_composite ON search_index(tenant_id, resource_type, resource_id, param_name, composite_group)",
        "CREATE INDEX IF NOT EXISTS idx_search_resource ON search_index(tenant_id, resource_type, resource_id)",
        "CREATE INDEX IF NOT EXISTS idx_search_token_display ON search_index(tenant_id, resource_type, param_name, value_token_display)",
        "CREATE INDEX IF NOT EXISTS idx_search_identifier_type ON search_index(tenant_id, resource_type, param_name, value_identifier_type_system, value_identifier_type_code)",
    ];

    for index_sql in &indexes {
        client
            .execute(*index_sql, &[])
            .await
            .map_err(|e| pg_error(format!("Failed to create index: {}", e)))?;
    }

    Ok(())
}

/// Create FTS (full-text search) tables using PostgreSQL tsvector/tsquery.
async fn create_fts_tables(client: &deadpool_postgres::Client) -> StorageResult<()> {
    // FTS table for resource narrative and full content
    client
        .execute(
            "CREATE TABLE IF NOT EXISTS resource_fts (
                resource_id TEXT NOT NULL,
                resource_type TEXT NOT NULL,
                tenant_id TEXT NOT NULL,
                narrative_text TEXT,
                full_content TEXT,
                narrative_tsvector TSVECTOR,
                content_tsvector TSVECTOR,
                CONSTRAINT fk_fts_resource FOREIGN KEY (tenant_id, resource_type, resource_id)
                    REFERENCES resources(tenant_id, resource_type, id) ON DELETE CASCADE
            )",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create resource_fts table: {}", e)))?;

    // GIN indexes for tsvector columns
    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_fts_narrative ON resource_fts USING GIN(narrative_tsvector)",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create narrative GIN index: {}", e)))?;

    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_fts_content ON resource_fts USING GIN(content_tsvector)",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create content GIN index: {}", e)))?;

    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_fts_lookup ON resource_fts(tenant_id, resource_type, resource_id)",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create FTS lookup index: {}", e)))?;

    // Create trigger function to automatically update tsvector columns
    client
        .execute(
            "CREATE OR REPLACE FUNCTION update_fts_vectors() RETURNS TRIGGER AS $$
            BEGIN
                NEW.narrative_tsvector := to_tsvector('english', COALESCE(NEW.narrative_text, ''));
                NEW.content_tsvector := to_tsvector('english', COALESCE(NEW.full_content, ''));
                RETURN NEW;
            END;
            $$ LANGUAGE plpgsql",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create FTS trigger function: {}", e)))?;

    // Create trigger (DROP first for idempotency)
    let _ = client
        .execute(
            "DROP TRIGGER IF EXISTS trg_update_fts_vectors ON resource_fts",
            &[],
        )
        .await;

    client
        .execute(
            "CREATE TRIGGER trg_update_fts_vectors
             BEFORE INSERT OR UPDATE ON resource_fts
             FOR EACH ROW EXECUTE FUNCTION update_fts_vectors()",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create FTS trigger: {}", e)))?;

    Ok(())
}

/// Run schema migrations from current version to latest.
async fn migrate_schema(
    client: &deadpool_postgres::Client,
    from_version: i32,
) -> StorageResult<()> {
    let mut version = from_version;

    while version < SCHEMA_VERSION {
        match version {
            1 => migrate_v1_to_v2(client).await?,
            2 => migrate_v2_to_v3(client).await?,
            3 => migrate_v3_to_v4(client).await?,
            4 => migrate_v4_to_v5(client).await?,
            5 => migrate_v5_to_v6(client).await?,
            6 => migrate_v6_to_v7(client).await?,
            7 => migrate_v7_to_v8(client).await?,
            8 => migrate_v8_to_v9(client).await?,
            9 => migrate_v9_to_v10(client).await?,
            10 => migrate_v10_to_v11(client).await?,
            11 => migrate_v11_to_v12(client).await?,
            12 => migrate_v12_to_v13(client).await?,
            13 => migrate_v13_to_v14(client).await?,
            14 => migrate_v14_to_v15(client).await?,
            _ => {
                return Err(pg_error(format!("Unknown schema version: {}", version)));
            }
        }
        version += 1;
        set_schema_version(client, version).await?;
    }

    Ok(())
}

/// v1 -> v2: Add new columns for enhanced search.
async fn migrate_v1_to_v2(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let migrations = [
        "ALTER TABLE search_index ADD COLUMN IF NOT EXISTS param_url TEXT",
        "ALTER TABLE search_index ADD COLUMN IF NOT EXISTS value_date_precision TEXT",
        "ALTER TABLE search_index ADD COLUMN IF NOT EXISTS value_quantity_system TEXT",
        "ALTER TABLE search_index ADD COLUMN IF NOT EXISTS composite_group INTEGER",
    ];

    for sql in &migrations {
        client
            .execute(*sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v1->v2 failed: {}", e)))?;
    }

    let indexes = [
        "CREATE INDEX IF NOT EXISTS idx_search_quantity ON search_index(tenant_id, resource_type, param_name, value_quantity_value, value_quantity_unit)",
        "CREATE INDEX IF NOT EXISTS idx_search_composite ON search_index(tenant_id, resource_type, resource_id, param_name, composite_group)",
        "CREATE INDEX IF NOT EXISTS idx_search_resource ON search_index(tenant_id, resource_type, resource_id)",
    ];

    for index_sql in &indexes {
        client
            .execute(*index_sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v1->v2 index creation failed: {}", e)))?;
    }

    Ok(())
}

/// v2 -> v3: Add FTS support.
async fn migrate_v2_to_v3(client: &deadpool_postgres::Client) -> StorageResult<()> {
    create_fts_tables(client).await
}

/// v3 -> v4: Add token display and identifier type columns.
async fn migrate_v3_to_v4(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let migrations = [
        "ALTER TABLE search_index ADD COLUMN IF NOT EXISTS value_token_display TEXT",
        "ALTER TABLE search_index ADD COLUMN IF NOT EXISTS value_identifier_type_system TEXT",
        "ALTER TABLE search_index ADD COLUMN IF NOT EXISTS value_identifier_type_code TEXT",
    ];

    for sql in &migrations {
        client
            .execute(*sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v3->v4 failed: {}", e)))?;
    }

    let indexes = [
        "CREATE INDEX IF NOT EXISTS idx_search_token_display ON search_index(tenant_id, resource_type, param_name, value_token_display)",
        "CREATE INDEX IF NOT EXISTS idx_search_identifier_type ON search_index(tenant_id, resource_type, param_name, value_identifier_type_system, value_identifier_type_code)",
    ];

    for index_sql in &indexes {
        client
            .execute(*index_sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v3->v4 index creation failed: {}", e)))?;
    }

    Ok(())
}

/// v4 -> v5: No-op for PostgreSQL (FTS triggers handled at creation time).
async fn migrate_v4_to_v5(_client: &deadpool_postgres::Client) -> StorageResult<()> {
    // PostgreSQL FTS triggers are created in create_fts_tables and handle
    // all fields including token display. No migration needed.
    Ok(())
}

/// v5 -> v6: Add bulk export and bulk submit tables.
async fn migrate_v5_to_v6(client: &deadpool_postgres::Client) -> StorageResult<()> {
    // Bulk Export tables
    client
        .execute(
            "CREATE TABLE IF NOT EXISTS bulk_export_jobs (
                id TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'accepted',
                level TEXT NOT NULL,
                group_id TEXT,
                request_json TEXT NOT NULL,
                transaction_time TIMESTAMPTZ NOT NULL,
                started_at TIMESTAMPTZ,
                completed_at TIMESTAMPTZ,
                error_message TEXT,
                current_type TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create bulk_export_jobs table: {}", e)))?;

    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_export_jobs_tenant ON bulk_export_jobs(tenant_id, status)",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create idx_export_jobs_tenant: {}", e)))?;

    client
        .execute(
            "CREATE TABLE IF NOT EXISTS bulk_export_progress (
                job_id TEXT NOT NULL,
                resource_type TEXT NOT NULL,
                total_count INTEGER,
                exported_count INTEGER DEFAULT 0,
                error_count INTEGER DEFAULT 0,
                cursor_state TEXT,
                PRIMARY KEY (job_id, resource_type),
                FOREIGN KEY (job_id) REFERENCES bulk_export_jobs(id) ON DELETE CASCADE
            )",
            &[],
        )
        .await
        .map_err(|e| {
            pg_error(format!(
                "Failed to create bulk_export_progress table: {}",
                e
            ))
        })?;

    client
        .execute(
            "CREATE TABLE IF NOT EXISTS bulk_export_files (
                id BIGSERIAL PRIMARY KEY,
                job_id TEXT NOT NULL,
                resource_type TEXT NOT NULL,
                file_type TEXT NOT NULL DEFAULT 'output',
                file_path TEXT NOT NULL,
                resource_count INTEGER DEFAULT 0,
                byte_count BIGINT DEFAULT 0,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                FOREIGN KEY (job_id) REFERENCES bulk_export_jobs(id) ON DELETE CASCADE
            )",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create bulk_export_files table: {}", e)))?;

    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_export_files_job ON bulk_export_files(job_id)",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create idx_export_files_job: {}", e)))?;

    // Bulk Submit tables
    client
        .execute(
            "CREATE TABLE IF NOT EXISTS bulk_submissions (
                tenant_id TEXT NOT NULL,
                submitter TEXT NOT NULL,
                submission_id TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'in-progress',
                created_at TIMESTAMPTZ NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL,
                completed_at TIMESTAMPTZ,
                metadata JSONB,
                PRIMARY KEY (tenant_id, submitter, submission_id)
            )",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create bulk_submissions table: {}", e)))?;

    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_bulk_submissions_status ON bulk_submissions(tenant_id, status)",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create idx_bulk_submissions_status: {}", e)))?;

    client
        .execute(
            "CREATE TABLE IF NOT EXISTS bulk_manifests (
                tenant_id TEXT NOT NULL,
                submitter TEXT NOT NULL,
                submission_id TEXT NOT NULL,
                manifest_id TEXT NOT NULL,
                manifest_url TEXT,
                replaces_manifest_url TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                added_at TIMESTAMPTZ NOT NULL,
                total_entries INTEGER DEFAULT 0,
                processed_entries INTEGER DEFAULT 0,
                failed_entries INTEGER DEFAULT 0,
                PRIMARY KEY (tenant_id, submitter, submission_id, manifest_id),
                FOREIGN KEY (tenant_id, submitter, submission_id)
                    REFERENCES bulk_submissions(tenant_id, submitter, submission_id) ON DELETE CASCADE
            )",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create bulk_manifests table: {}", e)))?;

    client
        .execute(
            "CREATE TABLE IF NOT EXISTS bulk_entry_results (
                tenant_id TEXT NOT NULL,
                submitter TEXT NOT NULL,
                submission_id TEXT NOT NULL,
                manifest_id TEXT NOT NULL,
                line_number INTEGER NOT NULL,
                resource_type TEXT NOT NULL,
                resource_id TEXT,
                created BOOLEAN,
                outcome TEXT NOT NULL,
                operation_outcome JSONB,
                PRIMARY KEY (tenant_id, submitter, submission_id, manifest_id, line_number),
                FOREIGN KEY (tenant_id, submitter, submission_id, manifest_id)
                    REFERENCES bulk_manifests(tenant_id, submitter, submission_id, manifest_id) ON DELETE CASCADE
            )",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create bulk_entry_results table: {}", e)))?;

    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_bulk_entry_results_outcome
             ON bulk_entry_results(tenant_id, submitter, submission_id, manifest_id, outcome)",
            &[],
        )
        .await
        .map_err(|e| {
            pg_error(format!(
                "Failed to create idx_bulk_entry_results_outcome: {}",
                e
            ))
        })?;

    client
        .execute(
            "CREATE TABLE IF NOT EXISTS bulk_submission_changes (
                tenant_id TEXT NOT NULL,
                submitter TEXT NOT NULL,
                submission_id TEXT NOT NULL,
                change_id TEXT NOT NULL,
                manifest_id TEXT NOT NULL,
                change_type TEXT NOT NULL,
                resource_type TEXT NOT NULL,
                resource_id TEXT NOT NULL,
                previous_version TEXT,
                new_version TEXT NOT NULL,
                previous_content JSONB,
                changed_at TIMESTAMPTZ NOT NULL,
                PRIMARY KEY (tenant_id, submitter, submission_id, change_id),
                FOREIGN KEY (tenant_id, submitter, submission_id)
                    REFERENCES bulk_submissions(tenant_id, submitter, submission_id) ON DELETE CASCADE
            )",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create bulk_submission_changes table: {}", e)))?;

    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_bulk_changes_resource
             ON bulk_submission_changes(tenant_id, resource_type, resource_id)",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create idx_bulk_changes_resource: {}", e)))?;

    Ok(())
}

/// v6 -> v7: Add FHIR version tracking.
async fn migrate_v6_to_v7(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let migrations = [
        "ALTER TABLE resources ADD COLUMN IF NOT EXISTS fhir_version TEXT NOT NULL DEFAULT '4.0'",
        "ALTER TABLE resource_history ADD COLUMN IF NOT EXISTS fhir_version TEXT NOT NULL DEFAULT '4.0'",
    ];

    for sql in &migrations {
        client
            .execute(*sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v6->v7 failed: {}", e)))?;
    }

    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_resources_fhir_version ON resources(tenant_id, fhir_version)",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Migration v6->v7 index creation failed: {}", e)))?;

    Ok(())
}

/// v7 -> v8: Add bulk-export worker/lease support.
async fn migrate_v7_to_v8(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let migrations = [
        "ALTER TABLE bulk_export_jobs ADD COLUMN IF NOT EXISTS worker_id TEXT",
        "ALTER TABLE bulk_export_jobs ADD COLUMN IF NOT EXISTS lease_expiry TIMESTAMPTZ",
        "ALTER TABLE bulk_export_jobs ADD COLUMN IF NOT EXISTS fencing_token BIGINT NOT NULL DEFAULT 0",
        "ALTER TABLE bulk_export_jobs ADD COLUMN IF NOT EXISTS heartbeat_at TIMESTAMPTZ",
        "ALTER TABLE bulk_export_jobs ADD COLUMN IF NOT EXISTS owner_subject TEXT",
        "ALTER TABLE bulk_export_jobs ADD COLUMN IF NOT EXISTS request_url TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE bulk_export_jobs ADD COLUMN IF NOT EXISTS fhir_version TEXT NOT NULL DEFAULT '4.0'",
        "ALTER TABLE bulk_export_files ADD COLUMN IF NOT EXISTS part_index INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE bulk_export_files ADD COLUMN IF NOT EXISTS fencing_token BIGINT NOT NULL DEFAULT 0",
    ];
    for sql in &migrations {
        client
            .execute(*sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v7->v8 failed: {}", e)))?;
    }

    // Backfill part_index: 0-based sequential per (job_id, file_type, resource_type).
    client
        .execute(
            "UPDATE bulk_export_files SET part_index = sub.rn FROM (
                SELECT id, ROW_NUMBER() OVER (
                    PARTITION BY job_id, file_type, resource_type ORDER BY id
                ) - 1 AS rn FROM bulk_export_files
             ) sub WHERE bulk_export_files.id = sub.id",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Migration v7->v8 backfill failed: {}", e)))?;

    let indexes = [
        "CREATE INDEX IF NOT EXISTS idx_export_jobs_claim
         ON bulk_export_jobs(tenant_id, status, lease_expiry)",
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_export_files_part
         ON bulk_export_files(job_id, file_type, resource_type, part_index)",
    ];
    for sql in &indexes {
        client
            .execute(*sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v7->v8 index failed: {}", e)))?;
    }

    Ok(())
}

/// Migrate from schema version 8 to version 9.
///
/// Adds the async Bulk Data Submit worker layer on top of the existing
/// synchronous bulk-submit ingestion tables (mirrors the SQLite v8->v9 migration).
async fn migrate_v8_to_v9(client: &deadpool_postgres::Client) -> StorageResult<()> {
    add_bulk_submit_worker_schema(client, "Migration v8->v9").await
}

async fn add_bulk_submit_worker_schema(
    client: &deadpool_postgres::Client,
    migration_label: &str,
) -> StorageResult<()> {
    let migrations = [
        // bulk_submissions: REST status + auth columns.
        "ALTER TABLE bulk_submissions ADD COLUMN IF NOT EXISTS owner_subject TEXT",
        "ALTER TABLE bulk_submissions ADD COLUMN IF NOT EXISTS poll_token TEXT",
        "ALTER TABLE bulk_submissions ADD COLUMN IF NOT EXISTS transaction_time TIMESTAMPTZ",
        "ALTER TABLE bulk_submissions ADD COLUMN IF NOT EXISTS requires_access_token BOOLEAN",
        "ALTER TABLE bulk_submissions ADD COLUMN IF NOT EXISTS request_url TEXT",
        // bulk_manifests: worker lease/fencing + kickoff parameters + resume cursor.
        "ALTER TABLE bulk_manifests ADD COLUMN IF NOT EXISTS worker_id TEXT",
        "ALTER TABLE bulk_manifests ADD COLUMN IF NOT EXISTS lease_expiry TIMESTAMPTZ",
        "ALTER TABLE bulk_manifests ADD COLUMN IF NOT EXISTS fencing_token BIGINT NOT NULL DEFAULT 0",
        "ALTER TABLE bulk_manifests ADD COLUMN IF NOT EXISTS fhir_base_url TEXT",
        "ALTER TABLE bulk_manifests ADD COLUMN IF NOT EXISTS output_format TEXT",
        "ALTER TABLE bulk_manifests ADD COLUMN IF NOT EXISTS file_request_headers TEXT",
        "ALTER TABLE bulk_manifests ADD COLUMN IF NOT EXISTS oauth_metadata_urls TEXT",
        "ALTER TABLE bulk_manifests ADD COLUMN IF NOT EXISTS file_encryption_key TEXT",
        "ALTER TABLE bulk_manifests ADD COLUMN IF NOT EXISTS last_processed_line BIGINT NOT NULL DEFAULT 0",
        "ALTER TABLE bulk_manifests ADD COLUMN IF NOT EXISTS import_directives TEXT",
        "ALTER TABLE bulk_manifests ADD COLUMN IF NOT EXISTS submission_metadata TEXT",
    ];
    for sql in &migrations {
        client
            .execute(*sql, &[])
            .await
            .map_err(|e| pg_error(format!("{} failed: {}", migration_label, e)))?;
    }

    client
        .execute(
            "CREATE TABLE IF NOT EXISTS bulk_submit_files (
                id BIGSERIAL PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                submitter TEXT NOT NULL,
                submission_id TEXT NOT NULL,
                manifest_url TEXT,
                file_type TEXT NOT NULL,
                resource_type TEXT,
                part_index INTEGER NOT NULL DEFAULT 0,
                fencing_token BIGINT NOT NULL DEFAULT 0,
                file_path TEXT NOT NULL,
                line_count BIGINT NOT NULL DEFAULT 0,
                byte_count BIGINT NOT NULL DEFAULT 0,
                count_severity TEXT,
                created_at TIMESTAMPTZ NOT NULL,
                FOREIGN KEY (tenant_id, submitter, submission_id)
                    REFERENCES bulk_submissions(tenant_id, submitter, submission_id) ON DELETE CASCADE
            )",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create bulk_submit_files: {}", e)))?;

    let indexes = [
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_bulk_submissions_poll_token
         ON bulk_submissions(poll_token)",
        "CREATE INDEX IF NOT EXISTS idx_bulk_manifests_claim
         ON bulk_manifests(tenant_id, status, lease_expiry)",
        "CREATE INDEX IF NOT EXISTS idx_bulk_submit_files_submission
         ON bulk_submit_files(tenant_id, submitter, submission_id)",
    ];
    for sql in &indexes {
        client
            .execute(*sql, &[])
            .await
            .map_err(|e| pg_error(format!("{} index failed: {}", migration_label, e)))?;
    }

    Ok(())
}

/// v9 -> v10: reference display, UCUM-canonical quantity columns,
/// and case/accent-folded string column.
async fn migrate_v9_to_v10(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let stmts = [
        "ALTER TABLE search_index ADD COLUMN IF NOT EXISTS value_reference_display TEXT",
        "CREATE INDEX IF NOT EXISTS idx_search_reference_display
         ON search_index(tenant_id, resource_type, param_name, value_reference_display)",
        "ALTER TABLE search_index ADD COLUMN IF NOT EXISTS value_quantity_canonical_value DOUBLE PRECISION",
        "ALTER TABLE search_index ADD COLUMN IF NOT EXISTS value_quantity_canonical_unit TEXT",
        "ALTER TABLE search_index ADD COLUMN IF NOT EXISTS value_string_folded TEXT",
        "CREATE INDEX IF NOT EXISTS idx_search_quantity_canonical
         ON search_index(tenant_id, resource_type, param_name, value_quantity_canonical_unit, value_quantity_canonical_value)",
        "CREATE INDEX IF NOT EXISTS idx_search_string_folded
         ON search_index(tenant_id, resource_type, param_name, value_string_folded)",
    ];
    for sql in stmts {
        client
            .execute(sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v9->v10 failed: {}", e)))?;
    }
    Ok(())
}

/// v10 -> v11: Add columns supporting `_contained` search. Index rows extracted
/// from a container's `contained[]` entries are flagged `is_contained = TRUE`
/// and carry the contained resource's type and local id; the row's
/// `resource_type` / `resource_id` continue to identify the container.
async fn migrate_v10_to_v11(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let stmts = [
        "ALTER TABLE search_index ADD COLUMN IF NOT EXISTS is_contained BOOLEAN NOT NULL DEFAULT FALSE",
        "ALTER TABLE search_index ADD COLUMN IF NOT EXISTS contained_type TEXT",
        "ALTER TABLE search_index ADD COLUMN IF NOT EXISTS contained_local_id TEXT",
        "CREATE INDEX IF NOT EXISTS idx_search_contained
         ON search_index(tenant_id, contained_type, is_contained, param_name)",
    ];
    for sql in stmts {
        client
            .execute(sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v10->v11 failed: {}", e)))?;
    }
    Ok(())
}

/// v11 -> v12: add async Bulk Data Submit worker schema for databases that
/// reached v11 through main before this feature branch was merged.
async fn migrate_v11_to_v12(client: &deadpool_postgres::Client) -> StorageResult<()> {
    add_bulk_submit_worker_schema(client, "Migration v11->v12").await
}

/// v12 -> v13: Add the `user_settings` table backing the per-user UI settings
/// store (theme, default tenant, active FHIR version, recent queries, …).
///
/// One opaque JSONB document is stored per user, keyed by `user_key`, with a
/// monotonic `version` for optimistic locking. This table is independent of the
/// FHIR `resources` table so UI preferences never leak into FHIR machinery.
async fn migrate_v12_to_v13(client: &deadpool_postgres::Client) -> StorageResult<()> {
    client
        .execute(
            "CREATE TABLE IF NOT EXISTS user_settings (
                user_key   TEXT PRIMARY KEY,
                data       JSONB NOT NULL,
                version    BIGINT NOT NULL DEFAULT 1,
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Migration v12->v13 failed: {}", e)))?;
    Ok(())
}

/// v14 -> v15: search performance indexes (issue #224).
///
/// Purely additive — nothing is dropped and no query semantics change. Every index
/// here was validated against a 1.45M-row replica of the benchmark dataset: each
/// one is measurably used by the plans the query builder now emits, and the set as
/// a whole took a 30-client mixed search workload from 45 tps / 659 ms to
/// 112 tps / 267 ms.
///
/// Deliberately NOT added: an index on
/// `(tenant_id, resource_type, param_name, resource_id, composite_group)`. It looks
/// like the obvious fix for the composite timeout, and it *is* required by the
/// correlated-`EXISTS` formulation of that query (which runs in ~1 ms) — but with
/// the SQL we actually emit, Postgres never scans it (0 scans over a clean 30 s
/// run; it BitmapOrs the token and quantity indexes instead). It indexes every row
/// of `search_index`, so it would be pure write amplification on the import path.
/// See `build_composite_condition` for why the `EXISTS` form was rejected.
///
/// Also deliberately NOT dropped: `idx_search_resource`. It reads as redundant (a
/// column prefix of `idx_search_composite`), but it is the per-resource probe in
/// the new plans and takes ~12M scans in a 30 s run — the hottest index in the
/// schema. It is also the write path's `DELETE FROM search_index WHERE
/// tenant/type/resource_id` and the FK cascade.
///
/// Index builds take a `SHARE` lock, blocking writes for their duration — measured
/// at ~6 s for this whole migration on 1.45M rows. Migrations run at startup before
/// the instance serves traffic, and `initialize_schema` holds an advisory lock, so
/// instances serialize rather than race. Operators upgrading a large database can
/// pre-build these `CONCURRENTLY` by hand; the `IF NOT EXISTS` clauses then make
/// this migration a no-op.
///
/// `CREATE INDEX CONCURRENTLY` is deliberately NOT used here: if the process dies
/// mid-build it leaves an `INVALID` index behind, and a later
/// `CREATE INDEX CONCURRENTLY IF NOT EXISTS` would see the name and skip it forever
/// — the index would silently never exist while the version marker claimed
/// otherwise.
async fn migrate_v14_to_v15(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let stmts = [
        // Ordered pagination.
        //
        // Every search ends `ORDER BY last_updated DESC, id ASC LIMIT n`, but no
        // index supplied that order: `idx_resources_type` is (tenant_id,
        // resource_type) and `idx_resources_updated` is (tenant_id, last_updated).
        // So a low-selectivity filter (`category=laboratory`, `date=gt2015`)
        // materialized every matching row and sorted it to return 20.
        //
        // Column directions must be declared explicitly: a backward scan of
        // (last_updated, id) yields `id DESC`, which does not match `id ASC`. The
        // reverse of this index exactly serves the keyset "previous" page.
        "CREATE INDEX IF NOT EXISTS idx_resources_search
         ON resources (tenant_id, resource_type, last_updated DESC, id ASC)
         WHERE is_deleted = FALSE",
        // Token search, code-first.
        //
        // `idx_search_token` orders (…, value_token_system, value_token_code), but
        // the common forms bind the code alone (`code=8302-2`, `category=laboratory`,
        // `status=finished`, `class=AMB`). With `system` ahead of it, a code-only
        // search has no leading equality and degrades to scanning the param slice.
        // Code-first makes those an equality seek; `system|code` still seeks on both.
        "CREATE INDEX IF NOT EXISTS idx_search_token_code
         ON search_index (tenant_id, resource_type, param_name, value_token_code, value_token_system)
         WHERE value_token_code IS NOT NULL",
        // String search (`Patient?name=`, `Patient?address=`, `Organization?name=`).
        //
        // The predicate is `COALESCE(value_string_folded, lower(value_string)) LIKE $n`.
        // A btree can only serve a prefix `LIKE` under `text_pattern_ops` (the
        // default opclass is collation-aware and cannot), and it must index the
        // whole COALESCE expression — a plain index on `value_string_folded` does
        // not match it. The COALESCE is load-bearing: `value_string_folded` was
        // added in v10 and is populated only on write, never backfilled, so rows
        // predating the upgrade have NULL there and must fall back to the raw column.
        //
        // `text_pattern_ops` also serves plain `=`, so this covers `:exact` too.
        "CREATE INDEX IF NOT EXISTS idx_search_string_folded_pattern
         ON search_index (tenant_id, resource_type, param_name,
                          (COALESCE(value_string_folded, lower(value_string))) text_pattern_ops)
         WHERE value_string IS NOT NULL",
        // Reference search and `_revinclude`.
        //
        // The predicate is `value_reference = $n OR value_reference LIKE $n || '/_history/%'`.
        // An OR is only index-usable when *every* arm is, and the default opclass
        // cannot serve the prefix LIKE — so the whole predicate degraded to a filter.
        "CREATE INDEX IF NOT EXISTS idx_search_reference_pattern
         ON search_index (tenant_id, resource_type, param_name, value_reference text_pattern_ops)
         WHERE value_reference IS NOT NULL",
    ];

    for sql in stmts {
        client
            .execute(sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v14->v15 failed: {}", e)))?;
    }

    // Multivariate statistics.
    //
    // `search_index` is one wide table holding every parameter of every resource
    // type, so `value_token_code` mixes LOINC codes, Encounter statuses, ActCodes
    // and so on in a single column. Every token search binds all three of
    // `resource_type`, `param_name` and `value_token_code`, and the three are
    // near-perfectly correlated — only Encounter rows have `param_name = 'status'`,
    // and only those carry `'finished'`. Postgres has no way to know that, so it
    // multiplies the three independent marginals and lands orders of magnitude low.
    //
    // Measured on the benchmark dataset: `Encounter?status=finished` matches ALL
    // 65,659 Encounters, but was estimated at 1,832 rows — a 36x under-estimate.
    // On that estimate the planner materializes every matching id, heap-fetches all
    // 65k rows to sort them, and returns 21: 5,261 ms. With the correlation
    // captured it instead walks `idx_resources_search` in `last_updated` order and
    // stops after ~21 rows: 10.4 ms, 4 buffers. A zero-match value
    // (`status=missing-status`, 0.05 ms) and a high-match control
    // (`category=laboratory`, 11.8 ms) are unaffected.
    //
    // This is why the plain ANALYZE tried in #224 had no measurable effect: no
    // amount of per-column statistics can express a cross-column correlation, and
    // the planner was already picking the only plan its estimate justified.
    //
    // The MCV list must span all three columns. A two-column
    // (param_name, value_token_code) object — which is what this migration
    // originally shipped — still leaves `resource_type` to be multiplied in
    // independently, and the estimate stays wrong.
    //
    // Best-effort: extended statistics need no special privilege, but a failure
    // here costs plan quality, not correctness, so it must not block startup.
    let stats = [
        // A wide MCV list: the table holds many (type, param, code) combinations and
        // the ones we must get right — ('Encounter','status','finished'),
        // ('Observation','category','laboratory') — have to survive in it.
        "ALTER TABLE search_index ALTER COLUMN value_token_code SET STATISTICS 2000",
        "ALTER TABLE search_index ALTER COLUMN param_name SET STATISTICS 1000",
        "CREATE STATISTICS IF NOT EXISTS stx_search_type_param_token (mcv, dependencies)
         ON resource_type, param_name, value_token_code FROM search_index",
        "CREATE STATISTICS IF NOT EXISTS stx_search_type_param (dependencies)
         ON resource_type, param_name FROM search_index",
        // Superseded by the three-column object above; harmless if it was never
        // created, and dropped so ANALYZE does not pay for it twice.
        "DROP STATISTICS IF EXISTS stx_search_param_token",
    ];
    for sql in stats {
        if let Err(e) = client.execute(sql, &[]).await {
            tracing::warn!(
                "Migration v14->v15: optional statistics step failed (plans may be \
                 suboptimal, search remains correct): {}",
                e
            );
        }
    }

    Ok(())
}

/// v13 -> v14: Add the tenant registry, mirroring the SQLite v14 migration.
///
/// A canonical list of first-class tenants backing the admin
/// tenant-maintenance API (list / add / delete). Until now a tenant was only
/// ever an implicit identifier string; this table records the tenants that
/// have been explicitly provisioned, with an optional human-friendly display
/// name and a creation timestamp. Tenants that merely have data but were never
/// registered are still discoverable via a `GROUP BY tenant_id` on
/// `resources`; the registry adds the metadata that data alone cannot provide.
///
/// `created_at` is stored as RFC 3339 TEXT (not TIMESTAMPTZ) so the registry
/// reads back byte-identically across backends.
async fn migrate_v13_to_v14(client: &deadpool_postgres::Client) -> StorageResult<()> {
    client
        .execute(
            "CREATE TABLE IF NOT EXISTS tenants (
                id           TEXT PRIMARY KEY,
                display_name TEXT,
                created_at   TEXT NOT NULL DEFAULT \
                    to_char(now() AT TIME ZONE 'utc', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
            )",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Migration v13->v14 failed: {}", e)))?;
    Ok(())
}

fn pg_error(message: String) -> crate::error::StorageError {
    crate::error::StorageError::Backend(BackendError::Internal {
        backend_name: "postgres".to_string(),
        message,
        source: None,
    })
}
