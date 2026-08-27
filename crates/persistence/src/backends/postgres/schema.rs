//! PostgreSQL schema definitions and migrations.

use crate::error::{BackendError, StorageResult};

/// Current schema version.
pub const SCHEMA_VERSION: i32 = 23;

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
                -- Slot-2 columns for the denormalized composite layout (#279).
                -- A composite instance is stored as ONE row carrying every
                -- component's value; 24 of the 46 R4 composites pair two
                -- components of the same type (almost all token+token, e.g.
                -- Observation.code-value-concept), which would otherwise
                -- collide. Slot 2 holds the second component of that type.
                -- Max observed per family is 2, so one extra slot suffices.
                value_token_system_2 TEXT,
                value_token_code_2 TEXT,
                value_number_2 DOUBLE PRECISION
                -- NOTE: there is deliberately no FOREIGN KEY to `resources`
                -- here. See `migrate_v22_to_v23` — every path that removes a
                -- resource row deletes this table's rows explicitly, and the
                -- constraint charged one extra SELECT plus a FOR KEY SHARE lock
                -- on the parent for every one of the ~39.5M index rows written.
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
        // The denormalized composite layout (#279) answers "code = X AND value > Y"
        // from one row, so these two partial covering indexes serve the whole
        // composite surface: 19 of the 46 R4 composites are token+quantity (all
        // three benchmark shapes) and 20 are token+token. Both are restricted to
        // composite rows, which is a small slice of search_index.
        "CREATE INDEX IF NOT EXISTS idx_search_composite_token_quantity ON search_index
         (tenant_id, resource_type, param_name, value_token_code, value_quantity_value)
         INCLUDE (resource_id)
         WHERE composite_group IS NOT NULL",
        "CREATE INDEX IF NOT EXISTS idx_search_composite_token_token ON search_index
         (tenant_id, resource_type, param_name, value_token_code, value_token_code_2)
         INCLUDE (resource_id)
         WHERE composite_group IS NOT NULL",
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
            15 => migrate_v15_to_v16(client).await?,
            16 => migrate_v16_to_v17(client).await?,
            17 => migrate_v17_to_v18(client).await?,
            18 => migrate_v18_to_v19(client).await?,
            19 => migrate_v19_to_v20(client).await?,
            20 => migrate_v20_to_v21(client).await?,
            21 => migrate_v21_to_v22(client).await?,
            22 => migrate_v22_to_v23(client).await?,
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
                file_url TEXT NOT NULL DEFAULT '',
                line_number INTEGER NOT NULL,
                resource_type TEXT NOT NULL,
                resource_id TEXT,
                created BOOLEAN,
                outcome TEXT NOT NULL,
                operation_outcome JSONB,
                PRIMARY KEY (tenant_id, submitter, submission_id, manifest_id, file_url, line_number),
                FOREIGN KEY (tenant_id, submitter, submission_id, manifest_id)
                    REFERENCES bulk_manifests(tenant_id, submitter, submission_id, manifest_id) ON DELETE CASCADE
            )",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create bulk_entry_results table: {}", e)))?;

    // #457 migration for pre-existing tables: line numbers restart per output
    // file, so the file belongs in the key — without it every file after the
    // first collided on its first entry. Pre-migration rows keep ''.
    client
        .execute(
            "ALTER TABLE bulk_entry_results ADD COLUMN IF NOT EXISTS file_url TEXT NOT NULL DEFAULT ''",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to add bulk_entry_results.file_url: {}", e)))?;
    client
        .execute(
            "DO $$
             BEGIN
               IF NOT EXISTS (
                 SELECT 1 FROM information_schema.key_column_usage
                 WHERE table_name = 'bulk_entry_results'
                   AND constraint_name = 'bulk_entry_results_pkey'
                   AND column_name = 'file_url'
               ) THEN
                 ALTER TABLE bulk_entry_results DROP CONSTRAINT bulk_entry_results_pkey;
                 ALTER TABLE bulk_entry_results ADD PRIMARY KEY
                   (tenant_id, submitter, submission_id, manifest_id, file_url, line_number);
               END IF;
             END $$",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to rekey bulk_entry_results: {}", e)))?;

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

/// The layout of `search_index`, recorded once at the v16 -> v17 migration.
///
/// v17 denormalizes the pagination sort key (`last_updated`) onto every index
/// row so that `ORDER BY last_updated DESC LIMIT n` can be answered from
/// `search_index` alone. Only the write path can populate it — the value belongs
/// to the resource, and a resource has many index rows — so an existing table
/// cannot be backfilled by SQL and its rows keep `last_updated IS NULL`.
///
/// The marker records which case a database is in, so the read path can use the
/// denormalized key only where every row actually has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexLayout {
    /// Every `search_index` row carries `last_updated`.
    Denormalized,
    /// Rows predate v17 and have no sort key; read paths must use the v16 form.
    Legacy,
}

/// Reads the recorded layout. Databases that never ran the v17 migration, and
/// any database whose marker is unreadable, are treated as [`IndexLayout::Legacy`]
/// — the conservative direction, since it selects the query form that works
/// against both layouts.
pub async fn read_index_layout(client: &deadpool_postgres::Client) -> IndexLayout {
    match client
        .query_opt("SELECT layout FROM search_index_layout LIMIT 1", &[])
        .await
    {
        Ok(Some(row)) if row.get::<_, String>(0) == "denormalized" => IndexLayout::Denormalized,
        Ok(_) => IndexLayout::Legacy,
        Err(e) => {
            tracing::warn!(
                "Could not read search_index layout marker, assuming legacy: {}",
                e
            );
            IndexLayout::Legacy
        }
    }
}

/// v16 -> v17: denormalize the pagination sort key onto `search_index`.
///
/// Every search ends `ORDER BY last_updated DESC, id ASC LIMIT n`, and that key
/// lived only on `resources`. For a filter that matches many rows the planner
/// therefore had to join every match before it could sort: measured on the
/// benchmark dataset, `Encounter?date=gt2010-01-01` joined 42,927 whole
/// resources (214,635 buffers, ~1.7 GB) to return 21 — 3,517 ms, of which the
/// index scan was 228 ms.
///
/// Extended statistics fixed the equivalent problem for token search in v15, but
/// they cannot fix this one: `value_date` selectivity is conditional on
/// `param_name`, and a range predicate is not expressible as an MCV entry. So
/// this removes the planner from the critical path instead of trying to inform
/// it — with the sort key on the index row, the top-n is resolved before
/// `resources` is touched at all.
///
/// Only the write path can populate the column, so an existing table is left
/// alone and marked `legacy`: it keeps the v16 query form and behaves exactly as
/// before. A fresh database is marked `denormalized`. (Promoting a populated
/// database, by re-extracting under `$reindex` and flipping the marker, is not
/// implemented here.)
async fn migrate_v16_to_v17(client: &deadpool_postgres::Client) -> StorageResult<()> {
    client
        .execute(
            "ALTER TABLE search_index ADD COLUMN IF NOT EXISTS last_updated TIMESTAMPTZ",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Migration v16->v17 failed: {}", e)))?;

    client
        .execute(
            "CREATE TABLE IF NOT EXISTS search_index_layout (layout TEXT NOT NULL)",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Migration v16->v17 failed: {}", e)))?;

    // A fresh database runs every migration in sequence against an empty table,
    // so emptiness here is exactly "no row predates the column".
    let populated = client
        .query_one("SELECT EXISTS (SELECT 1 FROM search_index LIMIT 1)", &[])
        .await
        .map_err(|e| pg_error(format!("Migration v16->v17 failed: {}", e)))?
        .get::<_, bool>(0);

    let layout = if populated { "legacy" } else { "denormalized" };

    client
        .execute("DELETE FROM search_index_layout", &[])
        .await
        .map_err(|e| pg_error(format!("Migration v16->v17 failed: {}", e)))?;
    client
        .execute(
            "INSERT INTO search_index_layout (layout) VALUES ($1)",
            &[&layout],
        )
        .await
        .map_err(|e| pg_error(format!("Migration v16->v17 failed: {}", e)))?;

    if populated {
        tracing::info!(
            "search_index predates v17; keeping the v16 search form. Existing \
             deployments are unaffected and see no change in behaviour."
        );
        return Ok(());
    }

    // Carry the sort key in the index payload so the top-n is index-only. These
    // rebuild instantly here — the table is empty by definition of this branch.
    let index_stmts = [
        "DROP INDEX IF EXISTS idx_search_date",
        "CREATE INDEX IF NOT EXISTS idx_search_date
         ON search_index (tenant_id, resource_type, param_name, value_date)
         INCLUDE (resource_id, last_updated)
         WHERE value_date IS NOT NULL",
        "DROP INDEX IF EXISTS idx_search_token_code",
        "CREATE INDEX IF NOT EXISTS idx_search_token_code
         ON search_index (tenant_id, resource_type, param_name, value_token_code, value_token_system)
         INCLUDE (resource_id, last_updated)
         WHERE value_token_code IS NOT NULL",
        "DROP INDEX IF EXISTS idx_search_quantity",
        "CREATE INDEX IF NOT EXISTS idx_search_quantity
         ON search_index (tenant_id, resource_type, param_name, value_quantity_value, value_quantity_unit)
         INCLUDE (resource_id, last_updated)",
        // The denormalized composite indexes (#279) carry the key too, so a
        // composite page is resolved the same way.
        "DROP INDEX IF EXISTS idx_search_composite_token_quantity",
        "CREATE INDEX IF NOT EXISTS idx_search_composite_token_quantity ON search_index
         (tenant_id, resource_type, param_name, value_token_code, value_quantity_value)
         INCLUDE (resource_id, last_updated)
         WHERE composite_group IS NOT NULL",
        "DROP INDEX IF EXISTS idx_search_composite_token_token",
        "CREATE INDEX IF NOT EXISTS idx_search_composite_token_token ON search_index
         (tenant_id, resource_type, param_name, value_token_code, value_token_code_2)
         INCLUDE (resource_id, last_updated)
         WHERE composite_group IS NOT NULL",
    ];

    for sql in index_stmts {
        client
            .execute(sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v16->v17 failed: {}", e)))?;
    }

    Ok(())
}

/// v21 -> v22: stop paying for a surrogate key and a composite index nobody
/// asks about on rows that have no composite.
///
/// Search is now 1.6x off the best published server while import is 11.1x off,
/// so the remaining work is on the write path. Both changes here remove work
/// from every inserted row without changing what any query can find.
///
/// **`search_index.id BIGSERIAL PRIMARY KEY`.** Nothing reads it — no query in
/// the backend selects, filters, orders by, joins on, or returns it; rows are
/// addressed by (tenant_id, resource_type, resource_id), which is what the FK
/// cascade used too (the FK itself is gone as of v23). `ROW_COLUMNS` in the writer never mentions it. It is a surrogate
/// key that exists because the table was written with one by habit, and on run
/// 33013229956 its index was **963 MB with 0 scans** — as it was in every run
/// today. Each inserted row pays a sequence `nextval()`, a btree insert into
/// that 963 MB index, and 8 bytes of heap, roughly 60M times over an import.
/// Dropping the column drops all three. The table is left without a primary
/// key, which is correct here: the write path deletes a resource's rows and
/// reinserts them, so there is no uniqueness to enforce, and no logical
/// replication depends on a replica identity.
///
/// **`idx_search_composite` becomes partial.** It is
/// (tenant_id, resource_type, resource_id, param_name, composite_group) with no
/// predicate, so all ~60M rows are indexed and every insert pays into 3796 MB —
/// but `composite_group` is NULL for every row that is not part of a composite.
/// `build_composite_condition` emits `composite_group IS NOT NULL` literally
/// (query_builder.rs, asserted in its tests), so the planner can prove the
/// partial index usable for exactly the queries that want it. Non-composite
/// rows stop paying for it entirely.
///
/// Anything still probing this index *without* constraining `composite_group`
/// falls back to `idx_search_resource` (tenant_id, resource_type, resource_id),
/// a column prefix of it that v15 documents as the hottest index in the schema
/// — 289,929 scans on that run. So the fallback is an index scan, not a seq
/// scan.
///
/// Deliberately NOT dropped: `idx_search_composite_token_token`, 1571 MB at 0
/// scans in every run today. It is already partial on `composite_group`, and it
/// serves token-token composites that this benchmark never issues but real
/// callers do. Same rule as `idx_search_string` in v18 — zero scans here means
/// the benchmark misses the shape, not that nobody needs it.
async fn migrate_v21_to_v22(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let stmts = [
        // Drops the PRIMARY KEY constraint, its 963 MB index, and the sequence
        // with it.
        "ALTER TABLE search_index DROP COLUMN IF EXISTS id",
        "DROP INDEX IF EXISTS idx_search_composite",
        "CREATE INDEX IF NOT EXISTS idx_search_composite
         ON search_index (tenant_id, resource_type, resource_id, param_name, composite_group)
         WHERE composite_group IS NOT NULL",
    ];

    for sql in stmts {
        client
            .execute(sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v21->v22 failed: {}", e)))?;
    }

    Ok(())
}

/// v22 -> v23: drop `fk_search_resource`.
///
/// `search_index` carried a composite FK to `resources` with `ON DELETE
/// CASCADE`. Postgres enforces that with a per-row `AFTER INSERT` trigger, and
/// `pg_stat_statements` for the import suite shows exactly what that costs:
///
/// ```text
/// exec_s  calls       query
///  500.3  39,516,106  SELECT 1 FROM ONLY "public"."resources" x WHERE ...
/// ```
///
/// 39.5M of the suite's 42.9M total statements — 92% — were this check, one per
/// index row inserted. The 500 s of execution time understates it: each also
/// pays trigger dispatch, its own snapshot, and a `FOR KEY SHARE` lock on the
/// parent row, which under concurrent writers turns into lock contention and
/// multixact WAL that this column does not attribute anywhere.
///
/// What the constraint bought was the guarantee that an index row cannot outlive
/// its resource. That guarantee does not actually rest on it: every path that
/// removes a `resources` row already deletes the matching `search_index` rows
/// first, in the same unit of work —
/// `purge` (one resource), `purge_all` (a type), and `purge_tenant_data` (a
/// tenant) each issue an explicit `DELETE FROM search_index` before the delete
/// from `resources`, and the update and soft-delete paths clear the index
/// through `delete_search_index`. The cascade never had anything left to do.
///
/// The obligation this moves onto the code is therefore already met, but it is
/// now load-bearing rather than belt-and-braces: **a new path that deletes from
/// `resources` must delete from `search_index` too.** The comments at those
/// call sites say so.
async fn migrate_v22_to_v23(client: &deadpool_postgres::Client) -> StorageResult<()> {
    client
        .execute(
            "ALTER TABLE search_index DROP CONSTRAINT IF EXISTS fk_search_resource",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Migration v22->v23 failed: {}", e)))?;

    Ok(())
}

/// v20 -> v21: restore the recent-first token index alongside v20's.
///
/// v20 dropped `idx_search_token_code_recent` on the reasoning that a value-first
/// key serves an equality predicate at every selectivity — true, and it fixed
/// `Observation?code` (1261 ms -> 255 ms p99). But it cost 50x on
/// `Observation?category` (38 ms -> 1693 ms) and 6x on `Encounter?status`, and
/// took the search suite from 1645 to 1065 RPS.
///
/// The isolated plan did not show it: single-value `category=laboratory` EXPLAINs
/// at 0.717 ms under v20, better than v19's 1.408 ms. The regression lives in
/// queries the plan capture did not model. The benchmark's token values include
/// comma lists —
///
/// ```text
/// category: "laboratory,vital-signs", "laboratory,vital-signs,survey"
/// code:     "8302-2,29463-7", "8480-6,8462-4,8867-4"
/// status:   "finished,in-progress", "finished,in-progress,planned"
/// ```
///
/// — roughly a fifth to a quarter of requests for those parameters, which is
/// precisely the tail a p99 reports. A comma list is an OR over several equality
/// tests. A recent-first index serves it as ONE stream already ordered by
/// `last_updated`, filtering the set membership as it goes, so the LIMIT still
/// stops at 22. A value-first key cannot: each value is its own ordered run, and
/// merging them into `last_updated DESC` order means sorting the whole match set
/// — ~500k rows for `category`.
///
/// So the two shapes are not redundant, they are complementary, and which one
/// wins depends on the predicate rather than on the data:
/// - one value, any selectivity -> value-first (v20's `idx_search_token_code`)
/// - several values, or a range -> recent-first (this index)
///
/// Both now exist and the planner picks per query. That is the same division v19
/// established for date and quantity, where the sparse range picks value-first
/// and the broad range picks recent-first — verified in plan sections V and U.
///
/// This re-adds 5.6 GB. The write path pays for it, which is a real cost on an
/// 11 GB host and the reason v20 was worth trying; the measurement says search
/// buys more than the write path loses.
async fn migrate_v20_to_v21(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let index_stmts = ["CREATE INDEX IF NOT EXISTS idx_search_token_code_recent
         ON search_index (tenant_id, resource_type, param_name, last_updated DESC, resource_id ASC)
         INCLUDE (value_token_code)
         WHERE value_token_code IS NOT NULL"];

    for sql in index_stmts {
        client
            .execute(sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v20->v21 failed: {}", e)))?;
    }

    Ok(())
}

/// v19 -> v20: fold the sort key into the token index instead of carrying a
/// second, recent-first copy of it.
///
/// v19 gave the fast path early termination by keying on
/// `(…, last_updated DESC, resource_id)` with the filter column as payload. For
/// a *range* predicate that is the only option: rows matching `value_date >= x`
/// are spread across the whole parameter slice, so nothing but the sort key can
/// lead the key. For an **equality** predicate it is the wrong trade, and run
/// 33003169681 shows what it cost:
///
/// ```text
/// idx_search_token_code_recent   18,522 scans   1,709,655,182 tuples read   5651 MB
/// idx_search_date_recent         19,459 scans       38,484,711 tuples read    463 MB
/// idx_search_quantity_recent     15,076 scans          948,762 tuples read    841 MB
/// ```
///
/// ~92,000 tuples read per scan against 1,977 and 63. A selective
/// `Observation?code=<rare LOINC>` walks ~92k index rows in `last_updated`
/// order before it collects 22 matches — the classic trap where `LIMIT` costing
/// assumes matches are spread uniformly along a scan ordered by something other
/// than the filtered column. It took that shape from 202 ms to 1261 ms p99.
///
/// An equality predicate does not have to choose. Putting the value ahead of
/// the sort key gives BOTH: the scan seeks straight to the one value, and
/// *within* that value the rows are already in `last_updated DESC, resource_id
/// ASC` order, so the LIMIT stops after 22 no matter how rare the code is.
/// Selectivity stops mattering, which is why this also removes the need for the
/// planner to choose between two token indexes at all — and the 5.6 GB
/// recent-first copy goes with it, which the write path gets back.
///
/// `value_token_system` moves to the payload. The `system|code` form filters it
/// from there during the same index-only scan; the `system|` form (system with
/// no code) is served by `idx_search_token`, which v18 documents and does not
/// touch.
///
/// Not applied to date or quantity: their predicates are ranges, and the two
/// index shapes v19 created remain the right answer there — the planner picks
/// value-first when the range is sparse (verified: plan section V) and
/// recent-first when it is broad (section U).
async fn migrate_v19_to_v20(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let index_stmts = [
        "DROP INDEX IF EXISTS idx_search_token_code",
        "CREATE INDEX IF NOT EXISTS idx_search_token_code
         ON search_index (tenant_id, resource_type, param_name, value_token_code,
                          last_updated DESC, resource_id ASC)
         INCLUDE (value_token_system)
         WHERE value_token_code IS NOT NULL",
        // Subsumed by the above at every selectivity, and the largest index in
        // the schema at 5651 MB.
        "DROP INDEX IF EXISTS idx_search_token_code_recent",
    ];

    for sql in index_stmts {
        client
            .execute(sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v19->v20 failed: {}", e)))?;
    }

    Ok(())
}

/// v18 -> v19: recent-first indexes so the fast path can terminate early.
///
/// Every search ends `ORDER BY last_updated DESC, id ASC LIMIT n`, and the
/// #279/v17 fast path takes that top-n from `search_index` directly:
///
/// ```sql
/// SELECT DISTINCT resource_id, last_updated FROM search_index
/// WHERE tenant_id = $1 AND resource_type = $2 AND <one membership test>
/// ORDER BY last_updated DESC, resource_id ASC LIMIT 22
/// ```
///
/// v17 put `last_updated` on those indexes as `INCLUDE (resource_id,
/// last_updated)`. **`INCLUDE` columns are payload, not key columns — they
/// cannot satisfy an `ORDER BY`.** So no index supplied the required order and
/// Postgres had to read every matching index row and sort it to find 22. v17
/// paid the storage for the column and got none of the ordering benefit, which
/// is why the shapes it targeted stayed slow (`Observation?date` p99 4075 ms,
/// `Observation?category` 4162 ms on run 32994869043) even after v18 freed
/// 13 GB of index footprint — ruling out cache pressure as the cause.
///
/// It also explains the regressions v17 introduced. Before v17 a shape like
/// `Encounter?class=AMB` could drive from `idx_resources_search`
/// (tenant_id, resource_type, last_updated DESC, id ASC), scanning `resources`
/// already in sort order and probing `search_index` per row, stopping after 22
/// — 18 ms. The fast path forced `search_index` to be the driver and replaced
/// that early termination with a full sort: 216 ms, a 12x regression.
///
/// These indexes put the sort key in the KEY, in the exact order the fast path
/// asks for, with the filter column as payload so the scan stays index-only.
/// Within a fixed (tenant_id, resource_type, param_name) — all bound by
/// equality — the remaining key order is precisely
/// `last_updated DESC, resource_id ASC`, so the `DISTINCT` becomes a streaming
/// `Unique` over presorted input and the `LIMIT` stops the scan.
///
/// The value-first indexes from v18 are KEPT alongside these: a *selective*
/// filter is still better served by seeking on the value and joining a handful
/// of rows, and a recent-first scan would have to walk the whole parameter
/// slice to find its 22 matches. Two shapes of index for two selectivity
/// regimes, with the planner choosing — rather than a gate in our SQL trying to
/// guess selectivity at build time, which it cannot see.
///
/// Partial, for the reason v18 gives: a row carrying a token value must not pay
/// an index insert into the date and quantity indexes.
async fn migrate_v18_to_v19(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let index_stmts = [
        "CREATE INDEX IF NOT EXISTS idx_search_date_recent
         ON search_index (tenant_id, resource_type, param_name, last_updated DESC, resource_id ASC)
         INCLUDE (value_date)
         WHERE value_date IS NOT NULL",
        "CREATE INDEX IF NOT EXISTS idx_search_token_code_recent
         ON search_index (tenant_id, resource_type, param_name, last_updated DESC, resource_id ASC)
         INCLUDE (value_token_code)
         WHERE value_token_code IS NOT NULL",
        "CREATE INDEX IF NOT EXISTS idx_search_quantity_recent
         ON search_index (tenant_id, resource_type, param_name, last_updated DESC, resource_id ASC)
         INCLUDE (value_quantity_value, value_quantity_unit)
         WHERE value_quantity_value IS NOT NULL",
    ];

    for sql in index_stmts {
        client
            .execute(sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v18->v19 failed: {}", e)))?;
    }

    Ok(())
}

/// v17 -> v18: make the value-column indexes partial.
///
/// `search_index` is one wide table holding every parameter of every resource
/// type, so for any given row almost every `value_*` column is NULL. An index
/// with no `WHERE` clause still carries an entry for every one of those NULL
/// rows, and — the part that actually costs — **every write pays an index
/// insert into every such index**. A row carrying one token value was inserting
/// into the number, quantity, canonical-quantity, uri, string, reference,
/// reference-display, token-display and identifier-type indexes as well.
///
/// Measured on run 32978436956 (689,080 resources): the index set totals 28 GB
/// against an 8.8 GB heap on an 11 GB Docker host. The schema already contained
/// the natural experiment — `idx_search_date`, partial since v16, is **344 MB**,
/// while `idx_search_quantity`, which has never carried a predicate, is
/// **5622 MB**. Same table, comparable population.
///
/// This is why import gains lagged so far behind crud (1.74x vs 4.24x) when
/// batching removed the write path's round trips in iteration 1: what remained
/// was index maintenance, and index maintenance was being paid on indexes the
/// row did not belong in.
///
/// Every predicate below is chosen so the planner can still prove the index
/// usable: `value_number = $1` (or any comparison, `LIKE`, or `ILIKE`) is strict
/// in its column and therefore implies `value_number IS NOT NULL`. No query
/// shape loses its index — including shapes this benchmark never exercises,
/// which is the whole reason these are rewritten as partial rather than dropped.
/// `idx_search_string` had 0 scans in that run and is *kept*: it is the only
/// index serving `:exact`, which the benchmark does not issue.
///
/// Deliberately NOT made partial:
/// - `idx_search_string_folded` (10,583 scans). The predicate is on
///   `COALESCE(value_string_folded, lower(value_string))`, and COALESCE is not
///   strict — it is non-NULL precisely when the folded column IS NULL and the
///   raw one is not. A `value_string_folded IS NOT NULL` predicate could not be
///   proved from it, so the index would silently stop being used.
/// - `idx_search_composite` (19,953 scans) and `idx_search_resource` (210,736
///   scans, the hottest index in the schema). Both key on `resource_id` with no
///   value column, and the per-resource probe does not constrain
///   `composite_group`, so a predicate on it would not be provable either.
///
/// Rebuilds take a `SHARE` lock. As with v15 and v17 these run at startup under
/// the `initialize_schema` advisory lock, before the instance serves traffic;
/// operators with a large existing database can pre-build the replacements
/// `CONCURRENTLY` by hand, after which `IF NOT EXISTS` makes this a no-op.
/// `CREATE INDEX CONCURRENTLY` is not used here for the reason given on v15: a
/// process death mid-build leaves an INVALID index that a later
/// `IF NOT EXISTS` would skip forever.
async fn migrate_v17_to_v18(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let index_stmts = [
        // The largest index in the schema, and the only one v17 rebuilt while
        // leaving its (absent) predicate alone — date and token_code kept theirs.
        // Every quantity predicate is a range comparison on the value.
        "DROP INDEX IF EXISTS idx_search_quantity",
        "CREATE INDEX IF NOT EXISTS idx_search_quantity
         ON search_index (tenant_id, resource_type, param_name, value_quantity_value, value_quantity_unit)
         INCLUDE (resource_id, last_updated)
         WHERE value_quantity_value IS NOT NULL",
        // `numeric_predicate("value_number", ..)` — comparisons only.
        "DROP INDEX IF EXISTS idx_search_number",
        "CREATE INDEX IF NOT EXISTS idx_search_number
         ON search_index (tenant_id, resource_type, param_name, value_number)
         WHERE value_number IS NOT NULL",
        // UCUM-canonical quantity: a range on the canonical value, optionally
        // with `value_quantity_canonical_unit = $n`.
        "DROP INDEX IF EXISTS idx_search_quantity_canonical",
        "CREATE INDEX IF NOT EXISTS idx_search_quantity_canonical
         ON search_index (tenant_id, resource_type, param_name, value_quantity_canonical_unit, value_quantity_canonical_value)
         WHERE value_quantity_canonical_value IS NOT NULL",
        // `=` or a prefix LIKE; the text_pattern_ops sibling is already partial.
        "DROP INDEX IF EXISTS idx_search_reference",
        "CREATE INDEX IF NOT EXISTS idx_search_reference
         ON search_index (tenant_id, resource_type, param_name, value_reference)
         WHERE value_reference IS NOT NULL",
        // The `system|` form binds the system alone; this index is what serves it.
        "DROP INDEX IF EXISTS idx_search_token",
        "CREATE INDEX IF NOT EXISTS idx_search_token
         ON search_index (tenant_id, resource_type, param_name, value_token_system, value_token_code)
         WHERE value_token_system IS NOT NULL",
        // `:exact` — `value_string = $n` against the bare column.
        "DROP INDEX IF EXISTS idx_search_string",
        "CREATE INDEX IF NOT EXISTS idx_search_string
         ON search_index (tenant_id, resource_type, param_name, value_string)
         WHERE value_string IS NOT NULL",
        // `=`, prefix LIKE, ILIKE, and the `:below` form `$1 LIKE value_uri || '%'`
        // — all strict in value_uri.
        "DROP INDEX IF EXISTS idx_search_uri",
        "CREATE INDEX IF NOT EXISTS idx_search_uri
         ON search_index (tenant_id, resource_type, param_name, value_uri)
         WHERE value_uri IS NOT NULL",
        // `value_token_display ILIKE $n`.
        "DROP INDEX IF EXISTS idx_search_token_display",
        "CREATE INDEX IF NOT EXISTS idx_search_token_display
         ON search_index (tenant_id, resource_type, param_name, value_token_display)
         WHERE value_token_display IS NOT NULL",
        // `value_reference_display ILIKE $n`.
        "DROP INDEX IF EXISTS idx_search_reference_display",
        "CREATE INDEX IF NOT EXISTS idx_search_reference_display
         ON search_index (tenant_id, resource_type, param_name, value_reference_display)
         WHERE value_reference_display IS NOT NULL",
        // `:of-type` binds the system, the code, or both, and either alone must
        // still imply the predicate — hence the disjunction rather than a
        // predicate on the leading column only.
        "DROP INDEX IF EXISTS idx_search_identifier_type",
        "CREATE INDEX IF NOT EXISTS idx_search_identifier_type
         ON search_index (tenant_id, resource_type, param_name, value_identifier_type_system, value_identifier_type_code)
         WHERE value_identifier_type_system IS NOT NULL OR value_identifier_type_code IS NOT NULL",
        // `_contained` search always binds `is_contained = TRUE`, and virtually
        // no row is a contained-resource row. The sibling lookup at the other
        // call site binds `is_contained = FALSE` but reads a different index.
        "DROP INDEX IF EXISTS idx_search_contained",
        "CREATE INDEX IF NOT EXISTS idx_search_contained
         ON search_index (tenant_id, contained_type, is_contained, param_name)
         WHERE is_contained",
    ];

    for sql in index_stmts {
        client
            .execute(sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v17->v18 failed: {}", e)))?;
    }

    Ok(())
}

/// v15 -> v16: covering indexes on token and date + wider MCV + date multivariate
/// stat (issue #281).
///
/// `idx_search_token_code` and `idx_search_date` are rebuilt with `INCLUDE (resource_id)`
/// so the subquery probes are index-only. `value_token_code` statistics are widened to
/// 4,000 and a matching multivariate stat is added for `value_date`, giving the planner
/// the same cross-column correlation data for date that token received in v14->v15.
async fn migrate_v15_to_v16(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let index_stmts = [
        "DROP INDEX IF EXISTS idx_search_token_code",
        "CREATE INDEX IF NOT EXISTS idx_search_token_code
         ON search_index (tenant_id, resource_type, param_name, value_token_code, value_token_system)
         INCLUDE (resource_id)
         WHERE value_token_code IS NOT NULL",
        "DROP INDEX IF EXISTS idx_search_date",
        "CREATE INDEX IF NOT EXISTS idx_search_date
         ON search_index (tenant_id, resource_type, param_name, value_date)
         INCLUDE (resource_id)
         WHERE value_date IS NOT NULL",
    ];

    for sql in index_stmts {
        client
            .execute(sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v15->v16 failed: {}", e)))?;
    }

    let stats_stmts = [
        "ALTER TABLE search_index ALTER COLUMN value_token_code SET STATISTICS 4000",
        "CREATE STATISTICS IF NOT EXISTS stx_search_type_param_date (mcv, dependencies)
         ON resource_type, param_name, value_date FROM search_index",
        "ANALYZE search_index",
    ];

    for sql in stats_stmts {
        if let Err(e) = client.execute(sql, &[]).await {
            tracing::warn!(
                "Migration v15->v16: optional statistics step failed (plans may be \
                 suboptimal, search remains correct): {}",
                e
            );
        }
    }

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
/// Also deliberately NOT dropped: `idx_search_token`. It reads as superseded by
/// the code-first `idx_search_token_code` added below, but the two serve
/// different shapes: `build_token_condition` emits `value_token_system = $n`
/// alone for the `system|` form, which has no leading equality on the code and
/// so cannot seek a code-first index — doubly so because `idx_search_token_code`
/// is partial on `value_token_code IS NOT NULL`. System-only token search has
/// this index or it has a scan.
///
/// Also deliberately NOT dropped: `idx_search_resource`. It reads as redundant (a
/// column prefix of `idx_search_composite`), but it is the per-resource probe in
/// the new plans and takes ~12M scans in a 30 s run — the hottest index in the
/// schema. It is also what serves the write path's and every purge path's
/// `DELETE FROM search_index WHERE tenant/type/resource_id`.
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
        // NOTE: this does NOT cover `:exact`. `text_pattern_ops` does serve plain
        // `=`, but this index is on the COALESCE *expression*, and `:exact` emits
        // `value_string = $n` against the bare column (build_string_condition),
        // which no expression index on a different expression can match. The
        // `:exact` shape is served by `idx_search_string` — which is why that
        // index is load-bearing rather than the redundant duplicate it appears
        // to be next to this one. Do not prune it.
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
