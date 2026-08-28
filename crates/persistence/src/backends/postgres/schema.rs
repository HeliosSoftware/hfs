//! PostgreSQL schema definitions and migrations.

use crate::error::{BackendError, StorageResult};

/// Current schema version.
pub const SCHEMA_VERSION: i32 = 34;

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
///
/// This is the version-1 index set, and it is deliberately left as written:
/// `create_schema_v1` is always followed by the whole migration chain, which
/// rebuilds, narrows or drops most of these against an empty table at no cost.
/// The chain is the single source of truth for the live index set — several
/// indexes named below (`idx_search_composite` in v26; `idx_search_reference`,
/// `idx_search_token_display` in v27) no longer exist by the time the schema is
/// current.
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
                content_tsvector TSVECTOR
                -- NOTE: there is deliberately no FOREIGN KEY to `resources`
                -- here. See `migrate_v26_to_v27`, and `migrate_v22_to_v23`
                -- for the same decision on `search_index`: every path that
                -- removes a resource row deletes this table's row explicitly,
                -- and the constraint charged one extra SELECT plus a
                -- `FOR KEY SHARE` lock on the parent for every full-text write.
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

    // UNIQUE, not just an index: one resource has one full-text row. The write
    // path upserts on exactly these columns (`FTS_UPSERT_SQL`), which needs the
    // uniqueness to have somewhere to conflict, and `search_text` /
    // `search_content` join `resources` to this table, where a duplicate row
    // would return the same resource twice in a `_text` / `_content` page.
    client
        .execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_fts_lookup ON resource_fts(tenant_id, resource_type, resource_id)",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Failed to create FTS lookup index: {}", e)))?;

    // No `BEFORE INSERT` trigger derives the tsvectors here any more, and none
    // may be added back: the writer supplies them directly and leaves
    // `narrative_text` / `full_content` unbound, so a trigger reading those
    // columns would overwrite both vectors with the tsvector of an empty
    // string. See `migrate_v24_to_v25`.
    let _ = client
        .execute(
            "DROP TRIGGER IF EXISTS trg_update_fts_vectors ON resource_fts",
            &[],
        )
        .await;

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
            23 => migrate_v23_to_v24(client).await?,
            24 => migrate_v24_to_v25(client).await?,
            25 => migrate_v25_to_v26(client).await?,
            26 => migrate_v26_to_v27(client).await?,
            27 => migrate_v27_to_v28(client).await?,
            28 => migrate_v28_to_v29(client).await?,
            29 => migrate_v29_to_v30(client).await?,
            30 => migrate_v30_to_v31(client).await?,
            31 => migrate_v31_to_v32(client).await?,
            32 => migrate_v32_to_v33(client).await?,
            33 => migrate_v33_to_v34(client).await?,
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

/// v23 -> v24: give `idx_search_token` the payload and the sort key that every
/// other token index already has, and make the folded-string pattern index
/// reachable.
///
/// ## The `system|code` token form is 22% of all Postgres execution in search
///
/// Run 33029355759, per-shape client latency (`search-points.json`, n=9707 each):
///
/// ```text
/// shape                        total     p50    p90     p99      max
/// token Encounter class        426.7s   10.9   75.4   461.8   3600.8
/// token Encounter status       104.6s    7.7   24.2    41.5     80.7
/// ```
///
/// `class` is 11.9% of the whole suite's latency against a 4.2% fair share, and
/// its distribution is not a shifted copy of `status`'s — it is bimodal.
/// **9.43% of `class` requests exceed 100 ms; 0.30% of `status` requests exceed
/// 50 ms and none exceeds 100.** 9.43% is 1/10.6, and `class` has 11 values in
/// `k6/searchConfig.js`: exactly one value is pathological, uniformly across the
/// whole two-minute window (the per-10s buckets are flat, so this is not a plan
/// flip part-way through the run).
///
/// The two parameters live in the same table, with the same slice shape and the
/// same index family. The only difference between them is the values the
/// benchmark sends:
///
/// ```text
/// status: finished | in-progress | planned | arrived | cancelled | ...  (all bare codes)
/// class:  AMB | EMER | ... | http://terminology.hl7.org/CodeSystem/v3-ActCode|AMB
///                          | http://terminology.hl7.org/CodeSystem/v3-ActCode|EMER
///                          | missing|class-code
/// ```
///
/// `status` has no system-qualified value. `class` has three, and one of them —
/// `v3-ActCode|AMB` — matches roughly two thirds of all Encounters.
///
/// A `system|code` value builds `value_token_system = $n AND value_token_code = $m`
/// (see `build_token_condition`), and that is the only predicate shape in the
/// benchmark strict in `value_token_system`, so it is the only one that can
/// reach this index. `pg_stat_statements` and `pg_stat_user_indexes` from the
/// same run agree on the consequence:
///
/// ```text
/// exec_s  calls   mean_ms   rows     (the top statement of the search suite)
///  314.5   6254    50.286   90027
///
/// index                          scans   tuples_read
/// idx_search_token                4321    58,376,095   -> 13,510 tuples per scan
/// idx_search_token_code          33325       332,061   ->     10 tuples per scan
/// idx_search_token_code_recent   12175     4,055,875   ->    333 tuples per scan
/// ```
///
/// 6254 calls is the expected count of every `system|code` request in the run
/// (`category` 2157 + `code` 1494 + `class` 2647 = 6298; pg_stat_statements
/// normalises the inlined `param_name` literal, so all three share one entry).
/// 4321 scans is the subset of those that reached this index, and 58.4M tuples
/// over 4321 scans reconciles to within 2% as `class|AMB` (~40k rows) +
/// `class|EMER` + the two `code|<loinc>` values, with `class|AMB` alone
/// contributing ~35M.
///
/// ## Why it costs ~330 ms
///
/// `idx_search_token` is `(tenant_id, resource_type, param_name,
/// value_token_system, value_token_code)` with **no payload and no sort key** —
/// the one token index v19 never rebuilt. So the fast path's
///
/// ```sql
/// SELECT DISTINCT resource_id, last_updated FROM search_index
/// WHERE ... AND (value_token_system = $3 AND value_token_code = $4)
/// ORDER BY last_updated DESC, resource_id ASC LIMIT 22
/// ```
///
/// cannot be index-only: `resource_id` and `last_updated` are not in the index,
/// so every matching row costs a heap fetch into a 6 GB heap, and the whole
/// match set must be sorted before the LIMIT can take 22. For `class|AMB` that
/// is ~40,000 random heap fetches and a 40,000-row sort to return 22 rows.
///
/// The planner chooses it because it believes the conjunction is selective:
/// `value_token_system` and `value_token_code` are estimated independently, and
/// a code all but determines its system, so `sel(code='AMB') *
/// sel(system='…v3-ActCode')` under-estimates by orders of magnitude, after
/// which a 40,000-row scan is costed as a handful of rows. The same arithmetic
/// explains why `Observation?category=<system>|laboratory` is *fast*: its slice
/// is 689,080 rows rather than 61,751, the same under-estimate still lands above
/// the crossover, and the planner picks `idx_search_token_code_recent` instead.
///
/// ## The fix, and why it does not depend on the planner
///
/// Add `last_updated DESC, resource_id ASC` as **key** columns — not `INCLUDE`,
/// which is payload and cannot satisfy an ORDER BY (v19). Every column ahead of
/// them is bound by equality in this predicate, so the remaining key order is
/// exactly the order the fast path asks for: the scan becomes index-only,
/// `DISTINCT` becomes a streaming `Unique`, and the LIMIT stops it at 22
/// however many rows the value actually matches. `class|AMB` goes from ~40,000
/// index tuples + ~40,000 heap fetches + a 40,000-row sort to ~22 index tuples
/// and no heap fetch.
///
/// This is deliberately *not* a statistics fix. An extended statistic on
/// (value_token_system, value_token_code) would correct the estimate and let the
/// planner move to `idx_search_token_code`, but it would leave the cliff in
/// place for every value whose estimate is still wrong. Making the index the
/// planner already prefers cheap removes the cliff instead of steering around
/// it.
///
/// **Write-side cost.** The row set is unchanged — the index stays partial on
/// `value_token_system IS NOT NULL`, ~1.6M of the 39.5M index rows (157 MB at
/// ~92 bytes per entry). Import already pays one insert per such row; what
/// changes is the entry width, +8 bytes for `last_updated` and ~40 for
/// `resource_id`, so ~157 MB -> ~240 MB. That is +83 MB against 20 GB of
/// indexes and no new index insert for the other ~37.9M rows. For scale, v21
/// re-added 5.6 GB.
///
/// ## Second change: the folded-string pattern index was unreachable
///
/// From the same run:
///
/// ```text
/// idx_search_string_folded          38694 scans   142,058,567 tuples   372 MB
/// idx_search_string_folded_pattern      0 scans             0 tuples    25 MB
/// ```
///
/// 3,671 tuples read per scan, and the statement they belong to is the second
/// most expensive in the suite (199.7 s over 29,121 calls, 14% of search
/// execution). String search matches
/// `COALESCE(value_string_folded, lower(value_string)) LIKE 'x%'`, and only
/// `idx_search_string_folded_pattern` can serve that as a range — the other
/// index is keyed on the bare column, so it can supply the
/// `(tenant_id, resource_type, param_name)` prefix and nothing more, then filter
/// the whole slice.
///
/// The pattern index went unused because it is partial on
/// `WHERE value_string IS NOT NULL` and nothing in the query implied that
/// predicate — the exact hazard v22's docstring flags as the reason *not* to put
/// a predicate on `idx_search_string_folded`. `build_string_condition` now emits
/// `value_string IS NOT NULL AND …` explicitly, which is sound because the
/// writer only ever sets `value_string_folded` alongside `value_string`
/// (`writer.rs`), so the added conjunct excludes no row the COALESCE could have
/// matched. `INCLUDE (resource_id, last_updated)` keeps the fast path's scan
/// index-only once it can seek.
///
/// `idx_search_string_folded` is **kept**: if the planner declines the pattern
/// index this change is a no-op rather than a collapse, and it is the only index
/// that can serve an ORDER BY on a string parameter. The wider payload is paid
/// on ~250k rows — the size of the partial set, inferred from the 25 MB
/// footprint — i.e. tens of megabytes.
async fn migrate_v23_to_v24(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let index_stmts = [
        "DROP INDEX IF EXISTS idx_search_token",
        "CREATE INDEX IF NOT EXISTS idx_search_token
         ON search_index (tenant_id, resource_type, param_name, value_token_system,
                          value_token_code, last_updated DESC, resource_id ASC)
         WHERE value_token_system IS NOT NULL",
        "DROP INDEX IF EXISTS idx_search_string_folded_pattern",
        "CREATE INDEX IF NOT EXISTS idx_search_string_folded_pattern
         ON search_index (tenant_id, resource_type, param_name,
                          (COALESCE(value_string_folded, lower(value_string))) text_pattern_ops)
         INCLUDE (resource_id, last_updated)
         WHERE value_string IS NOT NULL",
    ];

    for sql in index_stmts {
        client
            .execute(sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v23->v24 failed: {}", e)))?;
    }

    Ok(())
}

/// v24 -> v25: stop storing the text `resource_fts` only ever indexed.
///
/// `resource_fts` held four columns for two questions. `narrative_text` and
/// `full_content` carried the raw strings — the latter being every string value
/// in the resource, concatenated — and a `BEFORE INSERT` trigger derived
/// `narrative_tsvector` and `content_tsvector` from them. Only the two vectors
/// are ever read: `_text` and `_content` compile to
/// `narrative_tsvector @@ plainto_tsquery(...)` and `content_tsvector @@ ...`,
/// and nothing anywhere selects, filters or returns the text columns.
///
/// So every write stored the resource's text a second time — heap, TOAST
/// compression and WAL for it — purely to hand it to a trigger in the same
/// statement. `pg_stat_statements` for the crud suite of run 33029355759 puts
/// that insert at 793 s over 254,970 calls (2.8 ms mean), 11.5% of the suite's
/// entire Postgres time, second only to the search index inserts themselves.
///
/// The writer now computes both vectors inline —
/// `to_tsvector('english', $4)` — and binds nothing to the text columns. The
/// tokenising work is unchanged, and so is everything `_text` and `_content`
/// can find; what goes away is storing the input to it.
///
/// The trigger has to go with it, not merely become redundant: it assigns
/// `to_tsvector('english', COALESCE(NEW.full_content, ''))`, so left in place
/// against an unbound `full_content` it would overwrite the supplied vector
/// with the tsvector of an empty string and silently empty out `_content`
/// search.
///
/// The columns themselves are deliberately left in the table, on the same
/// reasoning as `param_url` in v22: `DROP COLUMN` runs against real databases
/// and cannot be undone, and unbound they cost one bit each in the null bitmap.
/// Rows written before this keep their text; nothing reads it either way.
async fn migrate_v24_to_v25(client: &deadpool_postgres::Client) -> StorageResult<()> {
    client
        .execute(
            "DROP TRIGGER IF EXISTS trg_update_fts_vectors ON resource_fts",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Migration v24->v25 failed: {}", e)))?;
    let _ = client
        .execute("DROP FUNCTION IF EXISTS update_fts_vectors()", &[])
        .await;

    Ok(())
}

/// v25 -> v26: stop indexing rows the index in question can never answer for.
///
/// `search_index` is one wide table holding every parameter of every resource
/// type, and the write path is where the whole benchmark lives: 79% of the
/// import suite's Postgres time and 63% of crud's is `INSERT INTO search_index`
/// (run 33029355759). That cost is per row, not per statement — the 13 batch
/// widths in that capture range from 12 to 128 rows per call and all land
/// between 130 and 210 µs per row — and `pg_stat_database` says where it goes:
/// 1.55 billion buffer hits for 39.5M inserted rows, **~39 buffer accesses per
/// row**, which is a heap insert plus roughly five btree descents. The lever is
/// therefore the *number of index insertions per row*, and the three indexes
/// below take one from rows they can never be read for.
///
/// ## Why not `PARTITION BY LIST (resource_type)`
///
/// Considered and rejected. It would remove no index insertion at all: a row
/// still enters the same number of indexes, each merely one or two levels
/// shallower, and the upper levels are the pages that are always cached. It also
/// does not hold up on its own premise —
/// `build_contained_condition` filters `contained_type`, not `resource_type`,
/// and the `:identifier` chain's inner `EXISTS` binds only `tenant_id` and
/// `param_name` — so those shapes would fan out to every partition. The main
/// path binds `resource_type = $2`, which prunes at execution time but still
/// takes a lock on every partition of the cached plan; ~150 R4 types times the
/// ~20 indexes here is ~3,000 relations per statement against a default
/// `max_locks_per_transaction` of 64. And converting a populated table is a
/// rewrite: 6 GB of heap plus 20 GB of index rebuilt inside
/// `initialize_schema`'s advisory lock, at startup, with every instance sharing
/// the database blocked behind it. Partial indexes get the same "one query
/// never sees the other population's rows" effect with no rewrite, no lock, and
/// no policy for resource types that appear at runtime.
///
/// ## 1. `idx_search_string_folded` becomes partial
///
/// It and `idx_search_resource` were the only two indexes on `search_index`
/// with no predicate, so all ~32M rows insert into it — while its
/// `text_pattern_ops` sibling, `idx_search_string_folded_pattern`, is partial on
/// `value_string IS NOT NULL` and measured **25 MB**. That is the size of the
/// set that actually has a value; the rest is ~32M index entries for rows whose
/// `value_string_folded` is NULL, one insert each, so that a string search on
/// `Patient.name` can find the ~250k that are not.
///
/// The predicate is reachable because v24 already made it so: it changed
/// `build_string_condition` to emit `value_string IS NOT NULL AND …` explicitly
/// (for the pattern index's benefit), and every remaining site that touches the
/// folded column does the same or uses a strict operator on `value_string`
/// (`:exact` is `value_string = $n`). Composite rows are the one case where
/// `value_string` is set and `value_string_folded` is not — `IndexRow::from_composite`
/// deliberately skips the folded column — and they stay in the index under this
/// predicate, exactly as they are today.
///
/// ## 2. The two composite covering indexes get their family's predicate
///
/// `idx_search_composite_token_quantity` and `idx_search_composite_token_token`
/// are partial on `composite_group IS NOT NULL` alone, so **every** composite
/// row inserts into **both** — 1738 MB and 1596 MB — although each serves one
/// family. 19 of the 46 R4 composites are token+quantity and 20 are token+token;
/// no row is both, so one of the two inserts each composite row pays is for an
/// index that cannot match it.
///
/// Adding the family column's `IS NOT NULL` keeps each index reachable without
/// touching the query builder, because `build_composite_component` already emits
/// predicates that imply it. A quantity component is always
/// `value_quantity_value <op> $n` and `<op>` is strict, and `predtest.c` proves
/// `x IS NOT NULL` from any strict operator clause over `x`. A token component
/// in slot 2 is `value_token_code_2 = $n`, or `value_token_system_2 = $n` for
/// the `system|` form, so that index's predicate is the disjunction of the two —
/// an OR predicate is proven when the clause implies either arm. The
/// disjunction selects the same rows as `value_token_code_2 IS NOT NULL` would
/// (`CompositeRow::place` never sets the system without the code); it is written
/// as an OR only so the `system|` spelling keeps the index.
///
/// The composite families with no covering index of their own — token+date,
/// token+string, token+number — are unaffected: they are served by
/// `idx_search_token_code`, which is partial on `value_token_code IS NOT NULL`
/// and is deliberately *not* narrowed here, so it remains the catch-all for
/// every composite shape as well as every plain token shape.
///
/// ## 3. `idx_search_composite` is dropped
///
/// v22 made it partial on `composite_group IS NOT NULL` and recorded the reason
/// it could be: anything probing it without constraining `composite_group`
/// "falls back to `idx_search_resource` (tenant_id, resource_type,
/// resource_id), a column prefix of it". That fallback is the whole story. After
/// v22 the index holds only composite rows, and its key still leads
/// `(tenant_id, resource_type, resource_id, …)`, so the only predicate that can
/// seek it is one that seeks `idx_search_resource` too — which is smaller, has
/// no predicate to prove, and is already the hottest index in the schema.
/// `build_composite_condition` seeks on `param_name`, the *fourth* column here,
/// so the composite search this index is named for cannot use it either. It has
/// **0 scans in every run on record** and 945 MB, and every composite row pays
/// an insert into it.
///
/// ## 4. `search_index` stops being auto-analyzed on every 10% of growth
///
/// v15 set `value_token_code`'s statistics target to 4000 (v16 raised it there
/// from 2000) to fix the token misestimate in #281. `ANALYZE` samples
/// `300 × statistics_target` rows, so 1.2M — more than the table has *blocks*,
/// which makes every analyze of `search_index` a full read of it. At the default
/// `autovacuum_analyze_scale_factor = 0.1` that fires on every 10% of growth, so
/// an import that grows the table from empty to 32M rows triggers on the order
/// of a hundred of them, each one reading a table that is on average a third of
/// its final size — an amount of buffer traffic comparable to the import's whole
/// 10.9M-block `blks_read`, on a 4 CPU / 11 GB host, competing with the inserts
/// that are the point of the exercise.
///
/// 0.4 keeps automatic statistics maintenance and cuts that to a handful. It
/// cannot affect plan quality for the measured suites: the benchmark runs an
/// explicit `VACUUM (ANALYZE)` after the load and before any search, and crud
/// changes 0.4% of a 32M-row table, which is under the threshold either way. The
/// statistics target itself is deliberately left at 4000 — lowering it is a
/// planner change, not a physical-design one, and #281 is why it is high.
///
/// ## What this costs a real database
///
/// Three of the four statements below are `DROP INDEX` and one `ALTER TABLE …
/// SET`, all catalog-only. The cost is the three `CREATE INDEX`es, each a full
/// scan of the ~6 GB heap holding a `SHARE` lock, so writes to `search_index`
/// block for the duration — minutes on a 32M-row table, and it happens at
/// startup under `initialize_schema`'s advisory lock. That is a real outage
/// window and is stated here rather than hidden; it is also the reason the
/// partitioned layout was rejected, since that variant is the same outage with a
/// 26 GB rewrite inside it instead of three scans. `CREATE INDEX CONCURRENTLY`
/// is not used, for the reason v15 gives: a process death mid-build leaves an
/// INVALID index that a later `IF NOT EXISTS` skips forever.
///
/// A pre-v17 database is left completely alone. Every predicate above is stated
/// in terms of the denormalized composite layout — one row per composite
/// instance — and a legacy database stores one row per *component*, where
/// `build_composite_condition_legacy` reads the base value columns of rows that
/// these predicates would exclude. Such a database keeps the v25 index set and
/// behaves exactly as before.
async fn migrate_v25_to_v26(client: &deadpool_postgres::Client) -> StorageResult<()> {
    if read_index_layout(client).await != IndexLayout::Denormalized {
        tracing::info!(
            "search_index predates v17; keeping the v25 index set. Existing \
             deployments are unaffected and see no change in behaviour."
        );
        return Ok(());
    }

    let stmts = [
        // 1. Only rows that have a string value can answer a string search.
        //    v24 made `build_string_condition` emit `value_string IS NOT NULL`.
        "DROP INDEX IF EXISTS idx_search_string_folded",
        "CREATE INDEX IF NOT EXISTS idx_search_string_folded
         ON search_index (tenant_id, resource_type, param_name, value_string_folded)
         WHERE value_string IS NOT NULL",
        // 2. One composite row belongs to one family; it should insert into one
        //    family's index.
        "DROP INDEX IF EXISTS idx_search_composite_token_quantity",
        "CREATE INDEX IF NOT EXISTS idx_search_composite_token_quantity ON search_index
         (tenant_id, resource_type, param_name, value_token_code, value_quantity_value)
         INCLUDE (resource_id)
         WHERE composite_group IS NOT NULL AND value_quantity_value IS NOT NULL",
        "DROP INDEX IF EXISTS idx_search_composite_token_token",
        "CREATE INDEX IF NOT EXISTS idx_search_composite_token_token ON search_index
         (tenant_id, resource_type, param_name, value_token_code, value_token_code_2)
         INCLUDE (resource_id)
         WHERE composite_group IS NOT NULL
           AND (value_token_code_2 IS NOT NULL OR value_token_system_2 IS NOT NULL)",
        // 3. Superseded by its own column prefix, `idx_search_resource`.
        "DROP INDEX IF EXISTS idx_search_composite",
        // 4. A full-table ANALYZE per 10% of growth is not statistics
        //    maintenance, it is a second workload.
        "ALTER TABLE search_index SET (autovacuum_analyze_scale_factor = 0.4)",
    ];

    for sql in stmts {
        client
            .execute(sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v25->v26 failed: {}", e)))?;
    }

    Ok(())
}

/// v26 -> v27: give the composite covering indexes the fast path's sort key —
/// as KEY columns — and give them back the payload v26 took away.
///
/// ## The regression this repairs
///
/// Composite is the search suite's remaining outlier. On run 33086933938
/// `pg_stat_statements` puts the two composite statements at 396.2 s / 22,190
/// calls (17.85 ms mean) and 116.4 s / 7,767 calls (14.99 ms) — a mean ~3x the
/// next shape and ~14x the reference shape. The two entries are the two
/// spellings `build_composite_component` emits for the token component:
/// `value_token_code = $n` for a bare code, and
/// `value_token_system = $n AND value_token_code = $m` for the `system|code`
/// form, which is why they are separate texts with a ~3:1 call split — exactly
/// the 21:6 bare:qualified split of the composite values in
/// `k6/searchConfig.js`.
///
/// Worse, `composite Observation combo-code-value-quantity` p99 went **88 ms ->
/// 114 ms** between runs 33029355759 (v25) and 33086933938 (v26), in absolute
/// terms, against an environment 1.58x friendlier. v26 is the only schema
/// change between them, and the cause is in its diff rather than its reasoning:
///
/// ```text
/// v17:  INCLUDE (resource_id, last_updated)   WHERE composite_group IS NOT NULL
/// v26:  INCLUDE (resource_id)                 WHERE composite_group IS NOT NULL
///                                               AND value_quantity_value IS NOT NULL
/// ```
///
/// v26's docstring argues only about the predicate; it never mentions the
/// payload. The `INCLUDE (resource_id)` text was copied from `create_indexes`
/// (the v1 shape) instead of from `migrate_v16_to_v17` (the shape actually in
/// the database), so `last_updated` was dropped silently. A single-parameter
/// composite search takes the v17 fast path —
/// `build_composite_condition` emits exactly one `id IN (SELECT resource_id
/// FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND …)`, which
/// `single_index_predicate` extracts (pinned by
/// `the_composite_fast_path_predicate_is_extractable`) — and that path runs
///
/// ```sql
/// SELECT DISTINCT resource_id, last_updated FROM search_index
/// WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '…'
///   AND composite_group IS NOT NULL
///   AND (value_token_code = $3) AND (value_quantity_value > $4)
/// ORDER BY last_updated DESC, resource_id ASC LIMIT 21
/// ```
///
/// `last_updated` is in that target list. Without it in the index the scan
/// cannot be index-only, so every matching row costs a heap fetch into the
/// ~6 GB heap — and a composite parameter's rows are scattered across a 22.6M
/// row table, so that is close to one random page per row. That is the 88 ->
/// 114 ms.
///
/// **So round 2 was half wrong, and only half.** Its predicate argument holds
/// and is kept verbatim below: `build_composite_component` emits
/// `value_quantity_value <op> $n` for every quantity spelling it has —
/// including the three-part `value|unit` form, which only adds a conjunct — and
/// `<op>` is always strict, so `predtest.c` proves `value_quantity_value IS NOT
/// NULL`; the token+token predicate is the disjunction of the two slot-2
/// spellings, and an OR predicate is proven from either arm. Verified for
/// `combo-code-value-quantity` specifically: all seven of its benchmark values
/// are `code$<op><number>`, so all seven emit a strict quantity operator. The
/// payload change that rode along with it is what regressed, and it is undone
/// here.
///
/// ## The general fix: stop sorting the match set
///
/// Restoring `last_updated` to `INCLUDE` would undo the regression and leave
/// the shape where v25 had it — still 17.85 ms, because payload is not
/// ordering. This is trap 6 and v19's own finding: **`INCLUDE` columns are
/// payload, not key columns, and cannot satisfy an `ORDER BY`.** With the sort
/// key only in the payload the plan must read *every* matching index row and
/// sort it to find 21. `code-value-quantity=8302-2$gt170` (body height over
/// 170 cm) matches a large fraction of a code slice that is tens of thousands
/// of rows; the LIMIT buys nothing.
///
/// v20 settled how to fix that for an equality predicate: put the value ahead
/// of the sort key and you get both — the scan seeks straight to the one value,
/// and *within* that value the rows are already in `last_updated DESC,
/// resource_id ASC` order, so `DISTINCT` becomes a streaming `Unique` and the
/// `LIMIT` stops the scan whatever the value's selectivity. A composite
/// token+quantity search is an equality on the token component and a range on
/// the quantity component, so the token component leads the key and the
/// quantity column moves to the payload, where the scan filters it without
/// touching the heap — the identical trade v20 made for `value_token_system` on
/// `idx_search_token_code`, and the reason that column is proven to work as an
/// index-only filter here.
///
/// `value_token_system` joins it in the payload, which is what fixes the second
/// statement (7,767 calls): the `system|code` form adds
/// `value_token_system = $n`, and with the column in the index that stays
/// index-only instead of falling back to a heap fetch per candidate row.
///
/// token+token gets the same treatment with *both* equality columns ahead of
/// the sort key. The benchmark never issues that family — 20 of the 46 R4
/// composites are in it and real callers do — so this is not measured here; it
/// is the same defect (v26 dropped its `last_updated` too) and the same repair.
///
/// ## The trade, stated plainly
///
/// Keying on the token value instead of on `(value_token_code,
/// value_quantity_value)` gives up the ability to seek past rows the quantity
/// range excludes. A value that matches a *large* code slice but almost none of
/// its rows — `29463-7$lt5` (body weight under 5 kg) or `2339-0$gt140` — now
/// walks that code's slice index-only rather than seeking to an empty range.
/// That is bounded: a code slice here is tens of thousands of entries, an
/// index-only walk of which is milliseconds, and it is bounded *better* than
/// what it replaces, because the value-first index sorts its whole match set on
/// every broad value — which is the common case and the measured 17.85 ms. The
/// worst case of the new shape is cheaper than the typical case of the old one,
/// which is why this replaces the index rather than adding a second one
/// alongside it.
///
/// Keeping both was considered and rejected on that arithmetic plus the write
/// side. It would also not work as intended: `value_quantity_value`'s histogram
/// is pooled across every parameter and every code in the table, so the planner
/// estimates `> 100` identically for every composite value and would pick one
/// plan for all of them regardless. There is no per-code selectivity for it to
/// choose on. Plan sections AM-AQ measure both regimes so the next round has
/// the numbers rather than this argument.
///
/// ## Write-side cost
///
/// **No new index and no new index insert.** Both indexes keep exactly the row
/// set v26 gave them — the same partial predicates, unchanged — so the number
/// of btree insertions per imported row is identical. What changes is the width
/// of an entry in two indexes that between them hold roughly 2.3M of the 22.6M
/// index rows (`combo-code-value-quantity` alone is 1,149,190):
///
/// - token+quantity: `+last_updated` 8 bytes as a key column, `resource_id`
///   moves from payload to key at the same width, `+value_quantity_value` 8
///   bytes and `+value_token_system` ~16 bytes of payload. ~+32 bytes on an
///   entry of ~100, so that index grows by roughly a third — tens of megabytes
///   against a 20 GB index set.
/// - token+token: `+last_updated` 8 bytes, `+value_token_system_2` payload,
///   `value_token_code_2` promoted from key position 5 to the same key at the
///   same width.
///
/// Neither index deduplicates today (btree deduplication is disabled for any
/// index with `INCLUDE` columns), so no compression is lost either. For scale,
/// v24 accepted +83 MB for the same shape of change on the token index and v21
/// re-added 5.6 GB.
///
/// The migration cost on a real database is two `CREATE INDEX`es over the
/// partial row sets, holding a `SHARE` lock on `search_index` — writes block
/// for their duration, at startup, under `initialize_schema`'s advisory lock.
/// That is smaller than v26's three full-heap builds because both are partial
/// over a ~10% slice, but it is an outage window and is stated rather than
/// hidden. `CREATE INDEX CONCURRENTLY` is not used, for v15's reason: a process
/// death mid-build leaves an INVALID index that a later `IF NOT EXISTS` skips
/// forever.
///
/// A pre-v17 database is left alone, exactly as v26 leaves it: every predicate
/// here is stated in terms of the denormalized one-row-per-composite-instance
/// layout, and `build_composite_condition_legacy` reads columns of rows these
/// predicates exclude.
async fn migrate_v26_to_v27(client: &deadpool_postgres::Client) -> StorageResult<()> {
    if read_index_layout(client).await != IndexLayout::Denormalized {
        tracing::info!(
            "search_index predates v17; keeping the v26 index set. The composite \
             fast path does not run against a legacy layout."
        );
        return Ok(());
    }

    let stmts = [
        // Equality on the token component leads; the fast path's sort key
        // follows as KEY columns so the scan terminates at the LIMIT; the range
        // column and the system column are payload the same scan filters on.
        "DROP INDEX IF EXISTS idx_search_composite_token_quantity",
        "CREATE INDEX IF NOT EXISTS idx_search_composite_token_quantity
         ON search_index (tenant_id, resource_type, param_name, value_token_code,
                          last_updated DESC, resource_id ASC)
         INCLUDE (value_quantity_value, value_token_system)
         WHERE composite_group IS NOT NULL AND value_quantity_value IS NOT NULL",
        // Both components are equalities here, so both lead the sort key.
        "DROP INDEX IF EXISTS idx_search_composite_token_token",
        "CREATE INDEX IF NOT EXISTS idx_search_composite_token_token
         ON search_index (tenant_id, resource_type, param_name, value_token_code,
                          value_token_code_2, last_updated DESC, resource_id ASC)
         INCLUDE (value_token_system, value_token_system_2)
         WHERE composite_group IS NOT NULL
           AND (value_token_code_2 IS NOT NULL OR value_token_system_2 IS NOT NULL)",
    ];

    for sql in stmts {
        client
            .execute(sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v26->v27 failed: {}", e)))?;
    }

    Ok(())
}

/// v27 -> v28: four indexes on `search_index` that no predicate the query
/// builder emits can seek, and that a surviving index already covers.
///
/// After v26 the write path is still 85% of the import suite's Postgres time —
/// 2,603.6 s of 3,063.8 s over 22,565,546 rows, 0.115 ms per row (run
/// 33086933938). Batching is finished (1,679 rows per statement; statement
/// count fell 42,875,393 -> 56,610), and rows per resource already fell
/// 24.2 -> 13.9. What is left is arithmetic: rows x indexes-each-row-enters x
/// cost per btree insert. v26 attacked the middle factor with partial
/// predicates. This attacks it again, by removing indexes outright — and
/// unlike v26 it is a catalog-only migration, because every drop is proved
/// against a surviving index rather than replaced by a rebuilt one.
///
/// The standard used here is the one v18 and v22 set and v26 restated: a
/// zero-scan index in this benchmark means the benchmark misses that shape,
/// not that nobody needs it. So none of the four below is dropped for being
/// cold. Each is dropped because the SQL this backend emits **cannot seek
/// it**, and the (tenant_id, resource_type, param_name) slice it would still
/// be scanned for is served by another index over a superset of its rows.
///
/// ## Index entries per row, before and after
///
/// Derived from `IndexRow::from_extracted` / `from_composite` (which columns a
/// row of each kind populates) crossed with every surviving index predicate.
///
/// ```text
/// row kind                     v26  v27
/// reference, with display        4    2
/// reference, no display          3    2
/// string                         4    3
/// token, with display            4    3
/// token, no display              3    3
/// composite token+string         6    5
/// date / quantity / uri / number      unchanged
/// composite token+token / +quantity   unchanged
/// ```
///
/// Reference rows are the largest single class left. The two changes that
/// deflated the table between them — dropping the duplicated
/// `CodeableConcept.text` row and keeping code-less text tokens out of the
/// composite cross-product — cut token and composite rows by a factor of two
/// to four and left reference rows untouched, and `_id` / `_lastUpdated` went
/// to zero. On the last full census `Provenance | target` alone is 1,626,336
/// rows, 1,626 per resource, all of them reference rows that carry no display:
/// 3 index entries each today, 2 after this.
///
/// ## 1. `idx_search_reference` — subsumed by `idx_search_reference_pattern`
///
/// The two indexes are the same key columns
/// (tenant_id, resource_type, param_name, value_reference) over the same
/// partial predicate (`value_reference IS NOT NULL`). The *only* difference is
/// the operator class on the last column: default `text_ops` here,
/// `text_pattern_ops` on the pattern index.
///
/// `text_pattern_ops` carries `=` at BTEqualStrategyNumber — text equality is
/// `texteq`, which is byte equality for any deterministic collation and is
/// therefore shared by both families. So every equality this backend emits on
/// `value_reference` (`value_reference = $n`, the `IN (subquery)` of
/// `_revinclude` and of `ChainQueryBuilder`'s links) seeks the pattern index
/// exactly as it seeks this one.
///
/// The converse does not hold, which is why the pattern index is the survivor:
/// `build_reference_condition` emits `value_reference LIKE $n || '/_history/%'`
/// and `ChainQueryBuilder` emits `value_reference LIKE 'Patient/%'`, and
/// Postgres can only turn a prefix `LIKE` into index bounds (`~>=~` / `~<~`)
/// inside the `text_pattern_ops` family. Dropping the pattern index instead
/// would strand those; dropping this one strands nothing.
///
/// What is genuinely given up is the collation ordering, which only a
/// MIN/MAX-to-index transform could have used — `sort_expression` emits
/// `(SELECT MIN(value_reference) … WHERE si.resource_id = resources.id AND
/// si.param_name = '…')`. That transform is a plan we do not want: the
/// correlated `resource_id` is not a key column here, so taking the minimum in
/// value order means walking the parameter's whole slice per outer row, where
/// `idx_search_resource` (tenant_id, resource_type, resource_id) answers the
/// same subquery with a three-column equality seek. Nothing else compares
/// `value_reference` with an ordering operator: there is no `ORDER BY`, no
/// range predicate and no merge condition on it anywhere in the backend.
///
/// ## 2. `idx_search_reference_display` and 3. `idx_search_token_display`
///
/// Every predicate this backend emits on either display column is `ILIKE`:
/// `value_reference_display ILIKE $n || '%'` and `… ILIKE '%' || $n || '%'`
/// (`build_reference_condition`), `value_token_display ILIKE $n` for `:text`
/// and `:code-text` (`build_token_condition`). A btree in the default operator
/// class cannot serve `ILIKE` at all — `match_pattern_prefix` derives bounds
/// for a case-insensitive pattern only when the fixed prefix contains no
/// letter, and even then only into a `text_pattern_ops` family, which neither
/// of these indexes has. Two of the three call sites additionally build the
/// pattern as `$n || '%'`, an `OpExpr` rather than a `Const`, so no prefix can
/// be read off them at all. These indexes have never been seekable and cannot
/// become so without a code change.
///
/// That leaves them as scanners of their leading (tenant_id, resource_type,
/// param_name) slice with the `ILIKE` as a filter, and for that the writer
/// gives a strict superset to scan instead:
///
/// - `IndexValue::Reference` sets `value_reference` whenever it sets
///   `value_reference_display`, and `from_composite` sets the reference without
///   a display, so every row of the display index is a row of
///   `idx_search_reference_pattern`, which shares its first three key columns.
/// - `IndexValue::Token` sets `value_token_code` unconditionally
///   (`Some(code.clone())`) and `from_composite` never sets a display, so every
///   row of the token-display index is a row of `idx_search_token_code`
///   (partial on `value_token_code IS NOT NULL`), which shares its first three
///   key columns.
///
/// So the `:text` / `:code-text` and reference-display shapes keep an index
/// scan over the same parameter slice; what they lose is one candidate for it.
///
/// ## 4. `idx_search_string_folded` — a key column nothing reads
///
/// `value_string_folded` appears in exactly one place in the emitted SQL:
/// inside `FOLDED_STRING_EXPR`, `COALESCE(value_string_folded,
/// lower(value_string))`. There is no bare predicate on the column anywhere —
/// not in the query builder, the chain builder, the contained path or
/// `sort_value_column`, which maps `String` to `value_string`. An index keyed
/// on the bare column cannot match a predicate on that COALESCE (v15 says so
/// about the reverse direction), so its fourth key column is unreachable by
/// construction.
///
/// Its first three key columns and its v26 predicate (`value_string IS NOT
/// NULL`) are identical to `idx_search_string_folded_pattern`'s, and that index
/// is both the one the string predicate actually seeks — v24 made it reachable
/// by having `build_string_condition` emit the `value_string IS NOT NULL`
/// conjunct — and strictly better for the scan, since v24 gave it
/// `INCLUDE (resource_id, last_updated)` and the fast path needs exactly those
/// two columns. `:exact` is unaffected: it emits `value_string = $n` against
/// the bare column and is served by `idx_search_string`, which v18 kept for
/// that reason and this does not touch.
///
/// ## Read-side risk, and how it is bounded
///
/// The one shape that changes plan rather than merely losing a redundant
/// candidate is a `_sort` on a reference parameter, discussed above, where the
/// alternative is the better plan. Everything else keeps an index whose leading
/// columns and population are a superset. No query loses a seek; no query falls
/// back to a sequential scan of `search_index`.
///
/// `.github/scripts/pg-search-plans.sql` proves this on real data rather than
/// on this argument: section S drops each index inside a transaction and
/// re-EXPLAINs the shape it served, so a run shows the actual plan with and
/// without it, and rolls back.
///
/// ## What this costs a real database
///
/// Four `DROP INDEX`, no `CREATE INDEX`. This is the first index migration
/// here that builds nothing: it is catalog-only, so there is no `SHARE` lock
/// held for the minutes a 22M-row index build takes and no full heap scan. Each
/// `DROP INDEX` does take a brief `ACCESS EXCLUSIVE` lock on `search_index`,
/// which waits behind any query already scanning the table and blocks new ones
/// while it waits — but it runs at startup under `initialize_schema`'s advisory
/// lock, before this instance serves traffic, and completes in milliseconds
/// once it has the lock. The space the four indexes occupied is returned to the
/// filesystem immediately rather than left as bloat.
///
/// Unlike v26 this is applied to a legacy (pre-v17) `search_index` too. Every
/// argument above is about which operator classes and which columns the emitted
/// SQL can use, and `build_composite_condition_legacy` shares
/// `build_composite_component` with the denormalized form — its string
/// component is the same non-sargable `value_string ILIKE $n`, and it reads no
/// display column and no folded column. Nothing here depends on the row layout.
async fn migrate_v27_to_v28(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let stmts = [
        // 1. Same key columns, same predicate, and `text_pattern_ops` carries
        //    `=`. The pattern index additionally serves the prefix `LIKE`s that
        //    this one never could.
        "DROP INDEX IF EXISTS idx_search_reference",
        // 2-3. `ILIKE` against a default-opclass btree is not sargable; the
        //    populations are strict subsets of indexes with identical leading
        //    columns.
        "DROP INDEX IF EXISTS idx_search_reference_display",
        "DROP INDEX IF EXISTS idx_search_token_display",
        // 4. Its fourth key column appears in no emitted predicate outside the
        //    COALESCE that only its `text_pattern_ops` sibling can match.
        "DROP INDEX IF EXISTS idx_search_string_folded",
    ];

    for sql in stmts {
        client
            .execute(sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v27->v28 failed: {}", e)))?;
    }

    Ok(())
}

/// v28 -> v29: make one resource's full-text row unique, and stop charging a
/// foreign-key check for writing it.
///
/// `INSERT INTO resource_fts` was the second-largest statement in the crud
/// suite on run 33086933938 — 4,225.7 s of Postgres execution time over 385,650
/// calls, **10.96 ms to insert a single row** — and it was preceded on every
/// update by a `DELETE FROM resource_fts` costing another 227.3 s. Neither the
/// delete nor the constraint has to be there.
///
/// ## `idx_fts_lookup` becomes UNIQUE
///
/// The table had no key, so a rewrite had to clear the old row before inserting
/// the new one. With a unique key the write becomes
/// `INSERT … ON CONFLICT (tenant_id, resource_type, resource_id) DO UPDATE`,
/// which replaces the row in place: one statement and one round trip instead of
/// two, on all 192,825 updates a 5-minute crud run performs. Measured locally
/// against Postgres 18 over 20,000 update-shaped operations on a preloaded
/// table, delete-then-insert took 2,947 ms and the upsert 2,158 ms — 27% less.
///
/// The index already existed on exactly these three columns, so uniqueness adds
/// no maintenance: the same b-tree, the same size, plus a check the insert's own
/// descent already pays for. It also closes a real hole. `search_text` and
/// `search_content` (`search_impl.rs`) `INNER JOIN resources` to
/// `resource_fts`, so a duplicated row returned the same resource twice in a
/// `_text` / `_content` page; nothing prevented one, because nothing ever
/// asserted a resource has one full-text row.
///
/// Duplicates are therefore deleted first — a database that has any cannot get
/// the unique index built otherwise. `ctid` picks the survivor: it is the
/// physical row identity, so `a.ctid < b.ctid` keeps exactly one row per key
/// without needing a column to order by.
///
/// ## `fk_fts_resource` goes
///
/// The same trade `migrate_v22_to_v23` made for `search_index`, for the same
/// reason: Postgres enforces a FK with a per-row `AFTER INSERT` trigger that
/// runs `SELECT 1 FROM ONLY resources … FOR KEY SHARE`, so every full-text write
/// paid an extra index probe, a `FOR KEY SHARE` lock on the parent row, and the
/// WAL record that lock writes. Measured locally, dropping it took 20,000
/// update-shaped operations from 6.5 to 5.5 WAL records each.
///
/// What the constraint bought — a full-text row cannot outlive its resource —
/// is upheld by code, exactly as `search_index`'s is. `resources` rows are hard
/// deleted in precisely three places (`purge`, `purge_all`,
/// `purge_tenant_data`), and all three already delete this table's rows first;
/// the `ON DELETE CASCADE` never had anything left to cascade to. A new deletion
/// path that skipped it would leave stale rows, which is why those call sites
/// carry the obligation in a comment.
///
/// ## What this costs a real database
///
/// A `DELETE` of duplicate rows (normally none), then a `DROP INDEX` and a
/// `CREATE UNIQUE INDEX` over one row per resource — a fraction of the
/// `search_index` rebuilds v26 performs — and a catalog-only
/// `ALTER TABLE … DROP CONSTRAINT`. It runs at startup under
/// `initialize_schema`'s advisory lock.
///
/// Rows written before v27 keep the lexemes they were built with, including the
/// ids and references `collect_strings` no longer indexes. That is a superset of
/// what the new writer produces, so nothing that used to be findable stops being
/// findable without a rewrite; `$reindex` rebuilds them on the narrower rule.
async fn migrate_v28_to_v29(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let stmts = [
        // 1. One row per resource, so the unique index can be built.
        "DELETE FROM resource_fts a
           USING resource_fts b
          WHERE a.ctid < b.ctid
            AND a.tenant_id = b.tenant_id
            AND a.resource_type = b.resource_type
            AND a.resource_id = b.resource_id",
        // 2. The upsert's conflict target.
        "DROP INDEX IF EXISTS idx_fts_lookup",
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_fts_lookup
         ON resource_fts (tenant_id, resource_type, resource_id)",
        // 3. A per-row trigger for a guarantee the deletion paths already give.
        "ALTER TABLE resource_fts DROP CONSTRAINT IF EXISTS fk_fts_resource",
    ];

    for sql in stmts {
        client
            .execute(sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v28->v29 failed: {}", e)))?;
    }

    Ok(())
}

/// v29 -> v30: the token family is 62% of the index footprint and one of its
/// three indexes is 2,283 MB of redundancy.
///
/// ## The regime this is trying to change
///
/// Run 33128380492, the Postgres arm of the FHIR benchmark, on the shared
/// 11 GB / 4 CPU Docker host:
///
/// ```text
/// all_indexes = 12 GB     heap = 3682 MB     search_index = 18.2M live rows (22.8M inserted)
/// ```
///
/// A ~15.7 GB working set on an 11 GB host is served from disk, end to end. It
/// is also the write path's whole bill: import is 84% one statement
/// (`INSERT INTO search_index`, 2,396.9 s over 22,565,546 rows = 0.106 ms/row),
/// and that cost is `rows x indexes-each-row-enters x cost per btree insert`.
/// Nothing else on the list is close: the three token indexes are 7,474 MB of
/// the 11.9 GB, 62% of the whole set.
///
/// ```text
/// index                          scans   tuples_read    size    tuples/scan
/// idx_search_token               16,869      152,092   2283 MB          9.0
/// idx_search_token_code          33,947      361,913   2945 MB         10.7
/// idx_search_token_code_recent   20,246    6,644,050   2246 MB          328
/// ```
///
/// ## What each of the three uniquely serves
///
/// `build_token_condition` emits exactly four predicate shapes, and `:not`
/// wraps the whole sublink in `NOT (...)` (no index either way):
///
/// ```text
/// S1  value_token_code = $n                                bare code, and the `|code` form
/// S2  value_token_system = $n                              the `system|` form
/// S3  (value_token_system = $s AND value_token_code = $c)   the `system|code` form
/// S4  value_token_display ILIKE $n                         :text / :code-text (index dropped in v27)
/// ```
///
/// Traced to the LIVE definitions — v20's for `idx_search_token_code`, v21's
/// for `idx_search_token_code_recent`, v24's for `idx_search_token`, NOT the v1
/// shapes in `create_indexes`, which is the mistake that cost v26 a 26 ms
/// regression:
///
/// - **S1** implies `value_token_code IS NOT NULL`, so it reaches both
///   code-partial indexes and neither system-partial one.
/// - **S2** implies `value_token_system IS NOT NULL` and nothing else. It is
///   the *only* shape that can prove `idx_search_token`'s predicate, and the
///   only one that cannot prove the other two indexes' predicate — so today it
///   has exactly one candidate.
/// - **S3** implies both predicates, so all three are candidates.
///
/// **S3 does not need a system-first index.** `idx_search_token_code` is
/// `(tenant_id, resource_type, param_name, value_token_code, last_updated DESC,
/// resource_id ASC) INCLUDE (value_token_system)`, and v20 built that payload
/// for precisely this: the scan seeks the code, the remaining key columns are
/// the fast path's `ORDER BY` so `DISTINCT` is a streaming `Unique` and the
/// `LIMIT` stops it, and the system is filtered from the payload without
/// touching the heap. Measured on a 4M-row local replica, `system|code` for a
/// pair that occurs:
///
/// ```text
/// today (idx_search_token):        Index Only Scan, 6 buffers, 22 rows, Heap Fetches 0
/// after (idx_search_token_code):   Index Only Scan, 6 buffers, 22 rows, Heap Fetches 0
///                                  Filter: value_token_system = $n, Rows Removed: 21
/// ```
///
/// Identical work. So `idx_search_token` earns its 2,283 MB on S2 alone.
///
/// ## Except it never actually served S2 well
///
/// `idx_search_token` is
/// `(tenant_id, resource_type, param_name, value_token_system, value_token_code,
/// last_updated DESC, resource_id ASC)`. v24 added the last two as KEY columns
/// and argued — correctly, for S3 — that "every column ahead of them is bound
/// by equality in this predicate". For **S2** that is false: `value_token_code`
/// sits at key position 5 and S2 does not bind it, so the sort key is not
/// reachable and the scan cannot be ordered. Measured, same replica, a `system|`
/// matching 66,667 rows:
///
/// ```text
/// Limit -> Sort (top-N heapsort) -> Index Only Scan using idx_search_token
///          rows=66667 scanned to return 22, 1074 buffers, 45.8 ms
/// ```
///
/// The whole match set is read and sorted. That is the exact pathology v24 was
/// written to remove, still present in the one shape this index exists for.
///
/// ## 1. `idx_search_token` is replaced by a seek-only index
///
/// ```text
/// idx_search_token_system (tenant_id, resource_type, param_name, value_token_system)
///                          WHERE value_token_system IS NOT NULL
/// ```
///
/// Same leading columns, same predicate, same rows — S2 keeps an exact seek on
/// the system. What goes is the payload and the sort key S2 could never use,
/// and dropping them is what makes the index small: with no `INCLUDE` column
/// and no `resource_id` in the key, every row of a
/// (tenant, type, param, system) group is a *duplicate key*, so btree
/// deduplication collapses the group to one key plus a posting list of 6-byte
/// TIDs. (`INCLUDE` disables deduplication outright, which is why the v20/v21
/// shapes get none of this.)
///
/// Measured on the replica — 4M token rows, 3.1M with a system (the 77.5% ratio
/// the real `idx_search_token`:`idx_search_token_code` size ratio implies),
/// realistic system URLs averaging 31 bytes, 36-char UUID `resource_id`. The
/// replica reproduces the three real indexes' relative geometry to within 5%
/// (real T1/T3 1.016 vs 1.021, real T2/T3 1.311 vs 1.252), so its ratios carry:
///
/// ```text
/// idx_search_token          (v24 shape)                445 MB
/// idx_search_token_system   (this)                      22 MB   = 4.9%
/// idx_search_token minus only value_token_code         416 MB   = 93.5%  (rejected, below)
/// ```
///
/// 2,283 MB x 4.9% = **~113 MB, a ~2,170 MB reduction**, 18% of the whole index
/// set. Deduplication degrades gracefully rather than cliff-edging: forced to
/// 1,000,420 distinct key groups over 3.1M rows — 3.1 rows per group, an absurd
/// worst case for a code system URI — the same index is 119 MB, still 27% of
/// what it replaces. Real terminology has a handful of systems per parameter.
///
/// **Narrowing instead of replacing was measured and rejected.** Removing only
/// `value_token_code` from the key (so S3 stops preferring this index but S2
/// keeps its streaming scan) is 416 MB against 445 — a 6.5% saving, because the
/// entry is dominated by the leading TEXT triple, the system URI and the 37-byte
/// `resource_id`, not by the code.
///
/// ## 2. The `system|` form gains the recent-first index as a second regime
///
/// A seek-only index means S2 heap-fetches every matching row and sorts them.
/// For a *selective* system that is the right plan and better than today
/// (measured: 300 matching rows, 15 buffers vs today's 25). For a *broad* one it
/// is not: the same 66,667-row query costs 60,064 buffers because every match is
/// a random heap fetch.
///
/// `build_token_condition` therefore now emits `value_token_code IS NOT NULL`
/// alongside the system equality for the `system|` form. That is the same device
/// v24 used to make `idx_search_string_folded_pattern` reachable, and it makes
/// `idx_search_token_code_recent` — `(tenant_id, resource_type, param_name,
/// last_updated DESC, resource_id ASC) INCLUDE (value_token_code)`, partial on
/// `value_token_code IS NOT NULL` — a legal candidate for S2. That index scans
/// the parameter slice already in the fast path's order, so the `LIMIT` stops it
/// at 22 matches whatever the system's selectivity:
///
/// ```text
/// broad system| (66,667 matches)   today: 1074 buffers / 45.8 ms   after: 26 buffers / 0.14 ms
/// rare  system| (300 matches)      today:   25 buffers            after: 15 buffers
/// ```
///
/// Both regimes are covered and the planner chooses: seek the system when it is
/// selective, stream recent-first when it is not — the division v19 and v21
/// established for date, quantity and token-code.
///
/// **Soundness of the added conjunct.** It excludes a row only if some row has
/// `value_token_system` set and `value_token_code` NULL. No such row can exist.
/// `IndexValue::Token` declares `code: String`, not `Option<String>`, and both
/// writer paths set the column unconditionally from it —
/// `IndexRow::from_extracted` (`row.value_token_code = Some(code.clone())`,
/// beside `row.value_token_system = system.clone()`) and `CompositeRow::place`
/// for both slots. An empty code is still a non-NULL empty string. The type
/// makes this true of every row any release of this backend has ever written,
/// which is why no data check accompanies the change.
///
/// ## The worst case, stated
///
/// After this, S3 is served by `idx_search_token_code`, which seeks the *code*
/// and filters the system. When the requested pair does not co-occur — a code
/// that is common under a different system than the one asked for — the scan
/// walks that code's whole slice instead of seeking to an empty range. Measured
/// on the replica for a 66,666-row code slice with no matching system: 999
/// buffers, 12.9 ms, index-only, no heap fetch. It is bounded by one code slice
/// and it costs no random I/O. That is the price of the 2,170 MB, and it is paid
/// only by a system/code combination that returns (nearly) nothing.
///
/// ## Write side
///
/// Every row that pays an insert today still pays one — the replacement is
/// partial on the same predicate — but it is a much cheaper insert, into an
/// index 20x smaller whose pages are posting lists rather than one entry per
/// row. A/B on the replica, 500,000 rows inserted (387,500 with a system) into a
/// table carrying only the old index and then only the new one:
///
/// ```text
/// v24 idx_search_token       535 MB WAL   5,245 ms
/// idx_search_token_system    390 MB WAL   3,834 ms      -27% both
/// ```
///
/// Both figures include the heap's own WAL and time, identical in both arms, so
/// the index's share of the saving is larger than 27%.
///
///
/// ## Entry width — measured, and deliberately NOT done here
///
/// The other half of the 12 GB is not redundancy, it is repetition. Every entry
/// in every index on `search_index` repeats the leading TEXT triple
/// `(tenant_id, resource_type, param_name)` — ~29 bytes for `default` /
/// `Observation` / `category` — and most of them carry `resource_id` as a
/// 36-char UUID **string**, 37 bytes, either in the key or in the payload. Both
/// were measured on the same replica rather than estimated:
///
/// ```text
/// idx_search_token_code with a TEXT resource_id           547 MB
///   … with a BIGINT surrogate instead                     423 MB   -22.7%
/// idx_search_token_code_recent with a TEXT resource_id    436 MB
///   … with a BIGINT surrogate instead                     304 MB   -30.3%
/// idx_search_token_code with a TEXT value_token_system    547 MB
///   … with a SMALLINT system id in the payload            431 MB   -21.2%
/// ```
///
/// After this migration the indexes carrying `resource_id` total ~8,503 MB
/// (`token_code` 2945 + `token_code_recent` 2246 + `string_folded_pattern` 495
/// + `quantity_recent` 482 + `resource` 473 + `date_recent` 452 + `quantity` 432
/// + `date` 386 + `composite_token_quantity` 357 + `composite_token_token` 235).
/// At the measured ~25% that surrogate is worth **~2.1 GB of index** plus ~530 MB
/// of heap (18.2M rows x 29 bytes) — the single largest remaining lever, larger
/// than this migration. Interning `value_token_system` is worth ~740 MB (~21% of
/// the ~3,537 MB that carry it) over a much smaller blast radius.
///
/// Neither is done here, and the reason is scope rather than doubt. A
/// `resource_id` surrogate has to be threaded through the writer (which must
/// learn each resource's surrogate before it can insert its index rows), every
/// `id IN (SELECT resource_id FROM search_index …)` sublink in the query builder
/// and the chain builder, the #279 fast path's
/// `SELECT DISTINCT resource_id, last_updated`, the `sort_expression`
/// subqueries that correlate `si.resource_id = resources.id`, the correlated
/// `EXISTS` in `build_reference_identifier_condition`, `delete_search_index`,
/// and the three purge paths — and a call site missed there returns silently
/// wrong results rather than an error. On an existing database it is also a full
/// rewrite of a 22M-row table with every index rebuilt behind it, which is a
/// different class of migration from the two catalog statements below. It is
/// written down here with its arithmetic so the next round can cost it honestly
/// instead of re-deriving it.
/// ## What this costs a real database
///
/// One `DROP INDEX` (catalog-only, returning 2,283 MB to the filesystem
/// immediately) and one `CREATE INDEX` over the `value_token_system IS NOT NULL`
/// slice. The build takes a `SHARE` lock on `search_index`, so writes block for
/// its duration — but it is the smallest index build in the chain's history by
/// an order of magnitude, and it runs at startup under `initialize_schema`'s
/// advisory lock, before the instance serves traffic. `CREATE INDEX
/// CONCURRENTLY` is not used, for the reason v15 gives: a process death
/// mid-build leaves an INVALID index that a later `IF NOT EXISTS` skips forever.
///
/// Unlike v26 and v27 this is applied to a legacy (pre-v17) `search_index` too.
/// All three token indexes were created or rebuilt by ungated migrations (v18,
/// v19, v20, v21, v24), so both layouts hold the identical token family, and the
/// writer invariant the new conjunct rests on is a property of `IndexValue`
/// rather than of the row layout.
async fn migrate_v29_to_v30(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let stmts = [
        // Its only unique shape is `system|`, and it never gave that shape the
        // sort key: `value_token_code` sits between the system and
        // `last_updated`, so the fast path sorted the whole match set anyway.
        "DROP INDEX IF EXISTS idx_search_token",
        // The seek `system|` actually uses, over the same rows. No INCLUDE and
        // no per-row key column, so btree deduplication collapses each
        // (tenant, type, param, system) group to a posting list.
        "CREATE INDEX IF NOT EXISTS idx_search_token_system
         ON search_index (tenant_id, resource_type, param_name, value_token_system)
         WHERE value_token_system IS NOT NULL",
    ];

    for sql in stmts {
        client
            .execute(sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v29->v30 failed: {}", e)))?;
    }

    Ok(())
}

/// v30 -> v31: v30's replacement index became an attractive nuisance, and the
/// recent-first token index cannot answer a `system|code` predicate without the
/// heap.
///
/// ## What v30 got right, and what it got wrong
///
/// Run 33176893776 against run 33128380492, same runner (`github-agent1`), so
/// the comparison is same-hardware:
///
/// ```text
///                 baseline      v30      ratio
/// all_indexes      12 GB      9605 MB    -2,395 MB   <- the point of v30
/// heap_only        3682 MB    3682 MB     unchanged
/// import           3.1 RPS    3.4 RPS     1.09x
/// crud             3629 RPS   3933 RPS    1.08x
/// search           2989 RPS   2541 RPS    0.85x      <- this migration
/// ```
///
/// The footprint claim landed slightly better than predicted:
/// `idx_search_token_system` measured **96 MB** against a predicted ~113 MB,
/// replacing 2,283 MB. The run's own census says why the deduplication estimate
/// held: 7,511,154 of 9,605,846 code-bearing rows carry a system (78.2%, against
/// the 77.5% the replica assumed) across **76 distinct
/// (resource_type, param_name, value_token_system) groups**, i.e. ~99,000 rows
/// per posting list. The write-side prediction landed too — import and crud
/// moved +9% and +8% together, which is one fewer btree insert per token row.
///
/// What v30 got wrong is one sentence of its own docstring: "S3 is served by
/// `idx_search_token_code`". It *can* be, and sections AD, AE and AF of the plan
/// capture show it being served exactly that way — `Index Only Scan using
/// idx_search_token_code`, `Heap Fetches: 0`, 5 buffers. But v30 never asked
/// whether the planner would *choose* it, and for one parameter it does not:
///
/// ```text
/// shape                        base p99   v30 p99
/// token Encounter class            26 ms    358 ms
/// ```
///
/// ## The measured mechanism
///
/// `pg_stat_user_indexes` from the v30 run:
///
/// ```text
/// index                          scans   tuples_read    size     tuples/scan
/// idx_search_token_system         5,334    80,089,347    96 MB      15,015
/// idx_search_token_code          39,003       415,842  2906 MB          10.7
/// idx_search_token_code_recent   17,516     3,361,223  2253 MB         191
/// ```
///
/// A 96 MB index with **no payload and no sort key** read 80 million tuples.
/// The only predicate shapes strict in `value_token_system` are `system|` (which
/// this benchmark never sends) and `system|code`, so those 5,334 scans are
/// `system|code` — a shape `idx_search_token_system` cannot answer without a
/// heap fetch for `value_token_code` on every row it returns, followed by a sort
/// of the whole match set, because it carries neither the code nor the fast
/// path's sort key.
///
/// Section AF2 of the same capture measures that plan directly, and prints the
/// reason the planner chooses it:
///
/// ```text
/// Index Scan using idx_search_token_system  (cost=0.56..110.84 rows=210)
///                                           (actual rows=67692) 67,651 buffers
/// -> Sort (top-N heapsort)                                      808 ms
/// ```
///
/// **rows=210 estimated, 67,692 actual — a 322x under-estimate.** That is the
/// same (system, code) independence error v24 documented; v24 neutralised it by
/// making the index the planner preferred *cheap*, at a cost of 2,283 MB, and
/// v30 removed that index without removing the error. `Encounter.class` is where
/// it bites because the parameter has ~10 distinct codes over one system, so
/// seeking the system alone returns essentially the whole slice. The p50 stayed
/// at 8 ms and only the p99 moved, which is the signature: the plan is fine when
/// the requested value is the common one and catastrophic when it is not.
///
/// Section AA shows the second-worst plan, chosen for `class=v3-ActCode|AMB`:
/// `Index Scan using idx_search_token_code_recent` with `Filter:
/// (value_token_system = ... AND value_token_code = ...)`. That index carries
/// `value_token_code` as payload but **not `value_token_system`**, so it is a
/// plain `Index Scan`, not an `Index Only Scan`: every candidate row costs a
/// random heap fetch into a 3.7 GB heap scattered across 22.6M rows just to
/// evaluate the system.
///
/// ## The fix: leave no reachable plan that touches the heap
///
/// Steering the planner was rejected. An extended statistic on
/// (`value_token_system`, `value_token_code`) would correct the 322x estimate,
/// but it is a bet on an estimate rather than a property of the schema, and v24
/// already recorded the objection: it "would leave the cliff in place for every
/// value whose estimate is still wrong". Instead, both changes below make the
/// *reachable* plans safe, so the estimate stops mattering.
///
/// **1. `idx_search_token_system` is dropped.** v30 created it one migration ago
/// and it is dropped here on its own measurement. It exists for `system|`, but
/// it is seekable by `system|code` too — both forms are strict in
/// `value_token_system` and there is no predicate that separates them — and for
/// `system|code` it is the worst plan in the family. Its 96 MB is not the point;
/// the 80 million heap-fetched tuples are.
///
/// This meets the drop rule v18/v22/v26/v28 set. The slice `system|` seeks it
/// for is served by `idx_search_token_code_recent` over a **superset** of its
/// rows: that index is partial on `value_token_code IS NOT NULL`, and every row
/// with a system has a code (`IndexValue::Token` declares `code: String`; pinned
/// by `a_token_row_never_has_a_system_without_a_code`), it shares the identical
/// leading three key columns, and change 2 below puts the system in its payload
/// so the scan is index-only. v30's `value_token_code IS NOT NULL` conjunct in
/// `build_token_condition` is what makes that index provable for the `system|`
/// form, and it becomes load-bearing here rather than merely helpful: without
/// it, `system|` would have no index at all.
///
/// **2. `idx_search_token_code_recent` gets `value_token_system` in its
/// payload.** Traced to its LIVE definition — v21's, verified against
/// `pg_indexes` on a database migrated through the whole chain, *not* the v1
/// shape in `create_indexes`:
///
/// ```text
/// (tenant_id, resource_type, param_name, last_updated DESC, resource_id ASC)
/// INCLUDE (value_token_code)  WHERE value_token_code IS NOT NULL
/// ```
///
/// The key is untouched. `last_updated DESC, resource_id ASC` are already KEY
/// columns and stay KEY columns — this migration adds a second `INCLUDE` column
/// and nothing else, so the streaming property v19 established and v27 had to
/// restore is not at risk here. Only the payload changes, and payload is exactly
/// what an index-only filter needs.
///
/// After both changes every predicate `build_token_condition` emits has only
/// index-only, LIMIT-terminating plans available:
///
/// ```text
/// shape        candidate                      plan
/// code=$c      idx_search_token_code          seek code, stream sort key, index-only
///              _code_recent                   stream, filter code from payload, index-only
/// system|code  idx_search_token_code          seek code, stream, filter system from payload
///              _code_recent                   stream, filter BOTH from payload  <- change 2
/// system|      _code_recent                   stream, filter system from payload  <- change 2
/// ```
///
/// Measured on a replica built to the run's real distribution — the
/// `Encounter/class` slice at full size (67,500 rows against the run's 67,692),
/// ~10 codes over one system, `value_token_code` at the production statistics
/// target of 4000:
///
/// ```text
///                              v30                          v31
/// system|code class|AMB    Index Scan token_system,      Index Only Scan token_code
///                          22,500 heap rows + Sort,      Heap Fetches 0, 5 buffers
///                          1,478 buffers
/// system|code class|EMER   same shape                    Index Only Scan, 5 buffers
/// system|code class|OBSENC same shape                    Index Only Scan, 5 buffers
/// broad system|            Index Scan + Sort,            Index Only Scan _code_recent,
///                          1,478 buffers                 Heap Fetches 0, 5 buffers
/// recent-first FORCED,     Index Scan, 407 heap          Index Only Scan,
///   class|EMER             fetches for the filter        Heap Fetches 0
/// ```
///
/// ## What this costs, and what it gives back
///
/// The `system|` form loses its exact seek. A system that matches little or
/// nothing now walks its parameter's slice instead of seeking to an empty range
/// — index-only, no heap, no random I/O, bounded by one `(resource_type,
/// param_name)` slice. On the replica that is 1,419 buffers and 14 ms for a
/// zero-match `class` system, against 4 buffers under v30. That is the honest
/// price, it is paid by a shape this benchmark does not send, and it is an order
/// of magnitude better than the 808 ms the index cost when the planner pointed
/// `system|code` at it.
///
/// Footprint: -96 MB for the drop, plus the width of `value_token_system` on
/// every row of a 2,253 MB index. The replica puts that at +27.9% (435 ->
/// 557 MB), but its entries are 115 bytes against the real index's 246, and the
/// added column is a fixed ~32 bytes either way, so the real growth is between
/// +13% (+293 MB) and the replica's +28% (+628 MB) — call it ~+360 MB. Net
/// against the 12 GB baseline this branch started from: still **-1.9 to
/// -2.2 GB**. Section AF4 of the plan capture prints the real number, and it now
/// prints `idx_search_token_code_recent`'s definition alongside it so the next
/// round can check the key columns from the artifact instead of from
/// `create_indexes`.
///
/// The write path keeps most of what v30 bought: the dropped index removes one
/// btree insert per system-bearing row, exactly as before, and change 2 widens
/// entries in an index those rows already entered rather than adding a new one.
///
/// ## What this migration deliberately does NOT touch
///
/// `quantity Observation value-quantity` also regressed on that run (27 ->
/// 261 ms p99). It is not this branch's. `pg-search-plans.sql` documents the
/// same shape alternating on identical code since run 33077075313, where it
/// normalised to 0.20x against this run's 0.12x; the AM1/AM2 captures are the
/// same index and the same node types in both runs with costs within 0.2%
/// (1327.71 vs 1327.01); `value_quantity_rows` is 512,311 and `table_rows`
/// 22,644,934 in **both**; and `param_name`'s `n_distinct` is 107 in both with
/// top frequencies equal to three decimal places. Every untouched index holds
/// its tuples-per-scan across the two runs to within 1% (string 3,119 vs 3,119,
/// composite 2,287 vs 2,299, reference 134.6 vs 135.3) — the sole exception
/// being `idx_search_quantity_recent`. Nothing in this branch writes a different
/// row or changes a statistic, so there is no mechanism by which it could move
/// that estimate. It is the documented instability and it is left alone.
///
/// ## What this costs a real database
///
/// One catalog-only `DROP INDEX` and one `DROP`/`CREATE` pair over the
/// `value_token_code IS NOT NULL` slice. The build takes a `SHARE` lock on
/// `search_index`, so writes block for its duration; it runs at startup under
/// `initialize_schema`'s advisory lock, before the instance serves traffic.
/// `CREATE INDEX CONCURRENTLY` is not used, for the reason v15 gives: a process
/// death mid-build leaves an INVALID index that a later `IF NOT EXISTS` skips
/// forever.
///
/// Applied to a legacy (pre-v17) layout too, for v30's reason: both token
/// indexes here were created by ungated migrations (v19, v20, v21), so both
/// layouts hold the identical shapes.
async fn migrate_v30_to_v31(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let stmts = [
        // 1. Seekable by `system|code`, which it cannot answer without a heap
        //    fetch per row and a sort. 5,334 scans, 80,089,347 tuples read.
        "DROP INDEX IF EXISTS idx_search_token_system",
        // 2. v21's key verbatim — `last_updated DESC, resource_id ASC` stay KEY
        //    columns — with `value_token_system` added to the payload so a
        //    `system|code` or `system|` filter is evaluated from the index
        //    instead of from the heap.
        "DROP INDEX IF EXISTS idx_search_token_code_recent",
        "CREATE INDEX IF NOT EXISTS idx_search_token_code_recent
         ON search_index (tenant_id, resource_type, param_name, last_updated DESC, resource_id ASC)
         INCLUDE (value_token_code, value_token_system)
         WHERE value_token_code IS NOT NULL",
    ];

    for sql in stmts {
        client
            .execute(sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v30->v31 failed: {}", e)))?;
    }

    Ok(())
}

/// v31 -> v32: store `value_reference` in its version-agnostic base form, and
/// delete the `LIKE` that existed only because it was not.
///
/// `build_reference_condition` emitted, for the `Type/id` form that is every
/// reference search the benchmark sends:
///
/// ```text
/// value_reference = $3 OR value_reference LIKE $4 ESCAPE '\'      -- $4 = 'Patient/<id>/\_history/%'
/// ```
///
/// On run 33128380492 that statement is the second-largest of the search suite —
/// **159.2 s of the ~885 s of real k6-window Postgres execution, 18%**, over
/// 109,981 calls at 1.447 ms, plus a three-value variant at 16.9 s / 5,304 calls
/// / 3.190 ms. The `LIKE` arm returned **one tuple in the entire run**.
///
/// ## Why the arm existed
///
/// `search/writer.rs` stored `Reference.reference` verbatim, so a resource
/// carrying `{"reference": "Patient/123/_history/4"}` put that whole string in
/// the column. FHIR reference search is version-agnostic, so a search for
/// `Patient/123` has to find it — and equality cannot, hence the pattern.
///
/// The search *value* was already normalized (`strip_reference_version`), and
/// has been since the parameter was written. This makes the other end match: the
/// writer stores the base, and the read becomes one equality. There is no third
/// form to cover, because the version suffix is the only thing the two ends
/// disagreed about.
///
/// ## What the arm cost, measured
///
/// A 3.4M-row local replica calibrated to the real slice geometry — 1,340,000
/// `Observation.subject` rows over 10,000 patients, i.e. the run's own 134
/// tuples per scan — on PostgreSQL 18.6, 300 searches over random patients,
/// warm cache, three repeats:
///
/// ```text
/// = OR LIKE (today)   1.496  1.525  1.510 ms/call
/// = only  (v32)       0.479  0.494  0.418 ms/call        -3.1x
/// ```
///
/// The saving is not the extra index descent, which is ~4 buffers of ~149. It
/// is the plan shape. An `OR` is index-usable only as a `BitmapOr`, so the whole
/// statement became `BitmapOr -> Bitmap Heap Scan`, which builds a bitmap and
/// then **re-checks the full disjunction on every heap tuple it visits** — a
/// `LIKE` match per row, 134 rows, to return 22:
///
/// ```text
/// Bitmap Heap Scan  Recheck Cond: ((… value_reference = '…') OR (… ~~ '…/\_history/%'))
///                   Filter: ((value_reference = '…') OR (value_reference ~~ '…'))
///                   Heap Blocks: exact=135
///   BitmapOr
///     Bitmap Index Scan on idx_search_reference_pattern   rows=134
///     Bitmap Index Scan on idx_search_reference_pattern   rows=1
/// ```
///
/// With one equality it is a plain `Index Scan`, no bitmap, no re-check. The
/// replica reproduces the run's per-call cost for this shape to within 4%
/// (1.50 ms vs 1.447 ms); it does **not** reproduce the three-value variant
/// (0.40 ms vs 3.190 ms), so the transferable quantity is the ratio between the
/// two arms on identical data, not the absolute number. The ratio is CPU-side,
/// and the benchmark host is 4 CPUs at ~43x concurrency.
///
/// Applying the ratio to the measured statement: 159.2 s -> ~51 s, about
/// **12% of the search suite's Postgres time**. That is a prediction, not a
/// result.
///
/// ## The generic-plan trap, which is real and is *not* what this run hit
///
/// A prefix `LIKE` against a bind parameter seeks a `text_pattern_ops` index
/// only under a **custom** plan. Under a generic plan Postgres derives no bounds
/// from `$n`, and because an `OR` needs every arm indexable, it loses the
/// *equality* arm too. Measured, same replica, same statement:
///
/// ```text
/// force_custom_plan    BitmapOr, 149 buffers                         2.89 ms
/// force_generic_plan   Bitmap Index Scan on (tenant, type, param)
///                      only, Filter removes 446,688 rows/worker,
///                      29,862 buffers                              193.58 ms   67x
/// ```
///
/// This backend does not hit that today: the search query builder emits
/// `format!`-built SQL through `client.query(&str, …)`, which re-prepares and
/// drops the statement every call, so every execution is a custom plan. The run
/// confirms it — 135,454 scans reading 1 tuple *total* is a working seek that
/// finds nothing, whereas a generic-plan fall-off would have read billions.
///
/// It is a landmine rather than a live defect, and the direction of travel walks
/// into it: `postgres/cached.rs` exists to route fixed SQL through
/// `prepare_cached`, and documents its own second rule as "only where a generic
/// plan is the plan anyway". A single equality satisfies that rule — measured,
/// the generic plan keeps the full four-column `Index Cond` — and the
/// disjunction never could.
///
/// ## Three readers that a stored version silently broke
///
/// This is not only a performance change. Every other consumer of
/// `value_reference` already assumed the base form and got the wrong answer for
/// a versioned one:
///
/// - `_revinclude` (`search_impl.rs`) builds `si.value_reference IN ($3, $4, …)`
///   from `format!("{}/{}", type, id)`. A stored `Patient/123/_history/4` never
///   matched, so a versioned reference was invisible to `_revinclude`.
/// - `ChainQueryBuilder` links on `si.value_reference IN (SELECT '{type}/' ||
///   resource_id …)` and extracts the target with `SUBSTRING(value_reference
///   FROM POSITION('/' IN value_reference) + 1)`, which yields
///   `123/_history/4` — matching no resource id.
/// - `build_reference_identifier_condition` uses the same `SUBSTRING`, with the
///   same result.
///
/// The SQLite backend reaches the same place from the other side: its
/// `build_identifier_condition` carries a `CASE WHEN INSTR(value_reference,
/// '/_history/') …` to strip the suffix at *read* time, and there is a test
/// pinning that `:identifier` must match a stored
/// `Patient/pat-shared/_history/1`. Postgres had no such device anywhere.
///
/// ## The data migration, and what happens to a database that skips it
///
/// One `UPDATE` over the rows that carry a suffix. It is a sequential scan of
/// `search_index` — `strpos(value_reference, '/_history/') > 0` has no usable
/// index bounds and never could, since the pattern is not anchored — so it is
/// one full read of the heap plus a row rewrite for each match. Measured on the
/// replica: **1.06 s over 3.4M rows / 561 MB**, ~530 MB/s, which puts the
/// benchmark's 3,682 MB heap at roughly 7 s. That is a one-off cost inside
/// `initialize_schema`'s advisory lock, an order of magnitude below the
/// `CREATE INDEX`es v15/v18/v24/v26 already take there, and on the benchmark it
/// is free: migrations run against an empty database before the import.
///
/// It is deliberately *not* limited to the denormalized layout — this is a
/// column value, not a row shape, and `build_composite_condition_legacy` reads
/// the same column.
///
/// The unmigrated combinations are all safe, in both directions:
///
/// - **v31 data, v32 code** cannot happen. `initialize_schema` runs the chain
///   under an advisory lock before the instance serves traffic, and a failure
///   aborts startup rather than continuing.
/// - **v32 data, v31 code** (a binary rollback) is safe because the pre-v32
///   predicate is a strict superset of the post-v32 one:
///   `= OR LIKE '<base>/\_history/%'` still matches a row holding `<base>`.
/// - A v32 database written by a *later* rollback to a pre-v32 writer would
///   store suffixes again, and a v32 reader would stop matching those rows.
///   That is the one lossy direction and it requires downgrading the binary,
///   writing, then upgrading; `set_schema_version` leaves the recorded version
///   at 32, so the `UPDATE` does not run again. It is called out here rather
///   than guarded against, because guarding it means keeping the `LIKE` arm,
///   which is the thing being removed.
///
/// ## The index is deliberately left alone
///
/// The obvious follow-through is to give `idx_search_reference_pattern` the fast
/// path's sort key, as v24 did for the token indexes: with the `OR` gone the
/// value predicate is a clean equality, so `(…, value_reference, last_updated
/// DESC, resource_id ASC)` would make `DISTINCT` a streaming `Unique`, stop the
/// scan at the `LIMIT`, and read 22 index tuples with **zero** heap fetches
/// instead of 134 index tuples plus 134 heap blocks.
///
/// It was measured and rejected. The reference index is small precisely because
/// it has no per-row column: `(tenant_id, resource_type, param_name,
/// value_reference)` repeats a value once per resource that references it, and
/// btree deduplication collapses each group to one key plus a posting list.
/// Adding `last_updated`/`resource_id` makes every key unique and turns
/// deduplication off. Measured on the replica, per-slice, same rows:
///
/// ```text
/// slice                      rows       dup    seek-only   with sort key
/// Observation.subject   1,340,000   134:1     7.9 B/row      160.4 B/row    20.3x
/// Observation.performer   500,000  1000:1     8.4 B/row      160.4 B/row    19.2x
/// Provenance.target       600,000     1:1   105.4 B/row      160.2 B/row     1.5x
/// whole reference set   2,440,200               76 MB           382 MB       5.0x
/// ```
///
/// The live index is 550 MB. The bound that follows from the two extreme
/// bytes-per-row figures is 5.5M-18.2M reference rows, so the sort-key shape
/// would be **880 MB to 2,912 MB** — between +330 MB and +2.4 GB on an 11 GB
/// host whose whole index set is 12 GB and where v30 has just spent a migration
/// recovering 2.2 GB. The upper half of that range gives back more than v30
/// gained, and nothing available here narrows it: the row census the run
/// captures would, and this seat did not have one.
///
/// `INCLUDE (resource_id, last_updated)` is the same trade, not a cheaper one —
/// 366 MB vs 382 MB on the replica, because included columns disable
/// deduplication as surely as key columns do, and it gives up the ordering as
/// well. Neither is done here.
async fn migrate_v31_to_v32(client: &deadpool_postgres::Client) -> StorageResult<()> {
    // `left(v, strpos(v, '/_history/') - 1)` rather than a `LIKE`/`substring`
    // pair: no escape character to get wrong (LIKE's default escape is the
    // backslash, so the pattern would have to be `'%/\_history/%'` for the
    // underscore to stay literal), and the predicate and the assignment then
    // share one function call.
    client
        .execute(
            "UPDATE search_index \
                SET value_reference = left(value_reference, strpos(value_reference, '/_history/') - 1) \
              WHERE value_reference IS NOT NULL \
                AND strpos(value_reference, '/_history/') > 0",
            &[],
        )
        .await
        .map_err(|e| pg_error(format!("Migration v31->v32 failed: {}", e)))?;

    Ok(())
}

/// v32 -> v33: make FHIR `string` search seek instead of scan.
///
/// String search was the single most expensive statement of run 33128380492:
/// **268.2 s of ~885 s** of k6-window Postgres execution — 30% of all search
/// work — over 45,150 calls at 5.94 ms mean, to return 125,551 rows. 2.8 rows
/// per call. `string Patient` is the worst published category we have: p99
/// **47 ms** against aidbox 23, microsoft 15, medplum 68, hapi 89.
///
/// `index-usage.txt` for the same run says where it went:
///
/// ```text
/// idx_search_string_folded_pattern    |      0 scans |           0 tuples | 495 MB
/// idx_search_string                   |  44931 scans | 140,153,204 tuples |  50 MB
/// ```
///
/// 3,120 tuples read per scan through a generic index, and the 495 MB index
/// `migrate_v23_to_v24` built *for this query* had zero scans for the third run
/// running. v28 dropped its competitor `idx_search_string_folded` on the theory
/// that the planner was merely preferring the smaller sibling; that did not
/// move it either. It has been pure write cost since v24.
///
/// ## Two independent reasons, both reproduced
///
/// Measured on a local 22.6M-row `search_index` built to the run's proportions
/// (110k rows with a string value, `(Patient, address)` slice 5,000 rows,
/// `(Patient, name)` 3,542 — the artifact's own numbers), with the v29 index set
/// and the index set as of v29 (this seat's baseline; the migration is
/// renumbered to v33 in the round-4 merge) and the v15/v16 extended statistics.
///
/// ### 1. The `IS NOT NULL` conjunct destroys the row estimate
///
/// v24 added `value_string IS NOT NULL` to `build_string_condition` so the
/// pattern index — partial on exactly that — could be *proved* usable, since
/// `COALESCE(a, b) LIKE …` does not imply it (COALESCE is not strict). That
/// reasoning was right, and the conjunct did make the index legal. But it is
/// also a selectivity factor, and the planner multiplies it in as if it were
/// independent of `param_name` when it is entirely determined by it:
///
/// ```text
/// EXPLAIN … WHERE resource_type='Patient' AND param_name='address'
///   ->  rows=5171          (truth 5000 — stx_search_type_param already fixes this pair)
/// EXPLAIN … same, AND value_string IS NOT NULL
///   ->  rows=23            (truth 5000 — x 0.0049, the table-wide non-null fraction)
/// ```
///
/// On a 25-row estimate every candidate looks free, the `LIMIT` looks instantly
/// satisfied, and the planner takes the physically smallest index that can
/// supply the `(tenant, type, param)` prefix — `idx_search_string`, 50 MB — and
/// filters. That is exactly the production plan (`search-plans.txt` section AI):
/// `cost=0.42..2.65 rows=1`, `Rows Removed by Filter: 4978`,
/// `Buffers: shared hit=4997 read=28`. ~40 MB of buffers touched to return 22
/// rows, 45,150 times in a five-minute window, on an 11 GB host — which is also
/// why this shape hurts every other category: it evicts their pages.
///
/// No extended statistics object fixes this. `(mcv, dependencies)` on
/// `resource_type, param_name, (value_string IS NOT NULL)` was built and
/// ANALYZEd at statistics targets 100 and 10000: the estimate went 23 -> 27.
/// Dependencies are equality-only, and an MCV list cannot help a range or
/// `LIKE` predicate over a high-cardinality value column.
///
/// So the conjunct is removed from the emitted SQL instead, and the index is
/// rebuilt with a predicate the *operator* implies: `~>=~` is strict, so
/// `COALESCE(…) ~>=~ $n` proves `COALESCE(…) IS NOT NULL` by itself. The
/// population is unchanged — `writer.rs` sets `value_string_folded` only
/// together with `value_string`, so `COALESCE(value_string_folded,
/// lower(value_string)) IS NOT NULL` and `value_string IS NOT NULL` select the
/// same rows. What changes is that the query no longer *mentions* the factor
/// the planner was mis-multiplying.
///
/// It also removes `idx_search_string` from the competition for this shape:
/// nothing in the new predicate implies *its* `value_string IS NOT NULL`
/// predicate, because COALESCE is not strict in `value_string`. `:exact`
/// (`value_string = $n`, strict) still reaches it, unchanged.
///
/// ### 2. A parameterized `LIKE` can never seek, at any estimate
///
/// `match_pattern_prefix` (`indxpath.c`) turns `LIKE` into an index range only
/// when it can see the pattern as a `Const`. The live statement binds it:
/// `… LIKE $3 ESCAPE $5`. Under a generic plan the whole predicate survives
/// rewriting as `~~ like_escape($1, '\')` — a function call on a parameter —
/// and no prefix exists to extract. Forced generic, same table, same index:
///
/// ```text
/// Index Cond: (tenant_id = … AND resource_type = … AND param_name = …)
/// Filter: (COALESCE(value_string_folded, lower(value_string)) ~~ like_escape($1, '\'))
/// Rows Removed by Filter: 4978          Buffers: shared hit=1667 read=53
/// ```
///
/// This is a latent hazard rather than today's plan: `search_impl.rs` calls
/// `client.query(&sql, …)` with a `&str`, and `ToStatementType::Query` routes
/// that to `prepare::prepare` — a fresh statement per call, closed on drop — so
/// the plan cache's custom-plan counter never reaches the five executions that
/// would switch it to generic. Reason 1 above is what is biting now. But that
/// is a property of one driver call site, not of the query, and `deadpool`'s
/// `prepare_cached` — used elsewhere in this backend, and the obvious thing to
/// reach for when trimming planning time — is one line away from turning the
/// latent case into the live one. `build_string_condition` therefore computes the bound in
/// Rust and emits the range itself: `FOLDED_STRING_EXPR ~>=~ $lo AND … ~<~ $hi`.
/// `~>=~`/`~<~` are the `text_pattern_ops` operators — bytewise, collation-free,
/// the same ones the planner generates — and UTF-8 byte order is code-point
/// order, so the range holds exactly the values with that prefix. Default FHIR
/// `string` semantics are starts-with, so this is the whole modifier.
///
/// ## Measured, paired, same database, after VACUUM ANALYZE
///
/// ```text
/// shape                                  before (v29 base)          after
/// Patient?address=Springfield (22)   1,709 buffers 1.07ms      25 buffers 0.09ms
/// Patient?name=Zzz       (0 matches) 1,183 buffers 1.15ms       4 buffers 0.10ms
/// Patient?family=Smith (whole slice)   725 buffers 1.79ms     733 buffers 1.65ms
/// Patient?address:contains=…         1,709 buffers 1.07ms   1,709 buffers (unchanged)
/// Patient?name:exact=…                   4 buffers          unchanged
/// ```
///
/// 68x and 296x fewer buffers on the two selective shapes, and the plan holds
/// under `plan_cache_mode = force_generic_plan`, which no `LIKE` form does.
///
/// The third row is the shape that gains nothing: a prefix matching the whole
/// parameter slice reads the same rows either way, just through a wider index.
/// It lands at parity *provided the index is compact* — repeated against a
/// deliberately bloated copy (fillfactor 10, 86 MB for the same 110k entries)
/// it is 1,012 buffers / 4.66 ms, a real regression. That is an argument for
/// the rebuild below, not against the seek.
///
/// Execution cost alone, 500 iterations inside a plpgsql loop so the plan is
/// cached and no client round trip or planning is counted: **513 ms -> 54 ms**,
/// 1.03 ms -> 0.11 ms per call. End to end through the wire with planning
/// included (pgbench, simple protocol, 4 clients, prefixes mixed 22-row /
/// 500-row / 0-row): 1,537 -> 2,186 tps. The gap between 9.5x and 1.4x is
/// planning and round trip on a box where the whole index is in RAM; the
/// benchmark host has 11 GB against a 15.7 GB working set, where the buffers
/// this stops touching are the point.
///
/// `:contains`/`:text` deliberately keep the old form. A leading `%` cannot
/// seek, so all the conjunct's removal would do is move an unavoidable
/// full-slice scan from the 50 MB index onto the wide covering one: measured
/// 1,709 -> 2,169 buffers, a 27% regression. It stays where it is. A trigram
/// GIN would be the real answer and is not affordable here — the benchmark host
/// has 11 GB of RAM against a 15.7 GB working set.
///
/// ## What this costs a real database
///
/// One `DROP INDEX` and one `CREATE INDEX` over the rows that have a string
/// value — 205,918 of 22,644,934 in the reference database (0.9%), the count
/// `search-plans.txt` section AL reports. Same key columns, same payload, same
/// population; only the predicate is reworded. The rebuild takes a `SHARE` lock
/// and runs at startup under `initialize_schema`'s advisory lock, before this
/// instance serves traffic, as v15/v17/v18/v24/v26 all do.
///
/// It also reclaims the index. 205,918 entries of ~130 bytes is ~27 MB; the run
/// measured **495 MB**, ~18x, which is churn bloat on an index that has never
/// been read and so has never had a scan clean anything up. The rebuild returns
/// that to the filesystem.
///
/// ## Measured on the real dataset: run 33179839720
///
/// The index is alive for the first time since v24 built it:
///
/// ```text
///                                    scans    tuples_read  tuples/scan
/// idx_search_string_folded_pattern  25,111         65,742          2.6   (was 0 / 0)
/// idx_search_string                 24,731     77,515,711        3,135   (was 44,931 / 140,153,204)
/// ```
///
/// `search-plans.txt` section AG went from 3,118 buffers / 35.5 ms to **4
/// buffers / 0.80 ms**; AI from 5,025 to 26 buffers. The `LIKE` statement fell
/// from 268.2 s over 45,150 calls to 101.2 s over 24,865, and the range
/// statement that replaced it does not appear in the top 15 at all.
///
/// ## The half that remains is `:contains`, and it is irreducible here
///
/// `idx_search_string` still took 24,731 scans at 3,135 tuples each, and the
/// shape p99 did not move (address 52 vs 51 ms). That is not a fallback and not
/// a missed case. `searchConfig.js` in the pinned benchmark repo
/// (HealthSamurai/fhir-server-performance-benchmark) declares
/// `"modifiers": ["", ":contains"]` for all three string shapes — `Patient.name`,
/// `Patient.address`, `Organization.name` — and `search.js` picks one per
/// request with `pickRand`. So **half of every string request is `:contains`**:
/// 50,112 string iterations x 0.5 = 25,056 expected, 24,865 observed, 0.8%
/// apart. p99 over a population that is half substring search is set entirely
/// by the substring half.
///
/// A leading `%` cannot seek a btree, so nothing in this migration or the next
/// one changes that. The real answer is a trigram GIN, and it does not fit: the
/// benchmark host has 11 GB of RAM against a 15.7 GB working set, and a
/// `gin_trgm_ops` index over 205,918 folded strings would be tens to hundreds
/// of megabytes of *additional* resident index competing with the working set
/// this change just relieved. Not worth it for a benchmark shape.
///
/// ## Why the v24 `INCLUDE (resource_id, last_updated)` is dropped
///
/// v24 added it to keep "the fast path's scan index-only once it can seek". It
/// can now seek, and the scan is still not index-only — in run 33179839720
/// (sections AG and AI), and locally on a table whose visibility map is 100%
/// all-visible. It is not a costing preference: with `enable_indexscan`,
/// `enable_bitmapscan` and `enable_seqscan` all off, so an index-only scan
/// would win by default, the planner still emits a plain `Index Scan` marked
/// `Disabled: true`.
///
/// The reason is structural. `check_index_only` collects the table attributes
/// the query needs with `pull_varattnos`, and a restriction clause on
/// `COALESCE(value_string_folded, lower(value_string))` pulls **both underlying
/// columns**. An expression index column contributes nothing to the set of
/// attributes an index can return (`indexkeys[i] == 0`), so the needed set can
/// never be a subset of the returnable set. Any expression index whose
/// expression appears in a `WHERE` clause is disqualified from index-only scans
/// by construction — which is exactly this index's only job.
///
/// So the payload is unreachable, and it is not free. `INCLUDE` columns disable
/// btree deduplication outright. Measured on a 205,918-row table shaped like the
/// real string population (half repetitive given/family/city values, half
/// near-unique street addresses and organization names):
///
/// ```text
/// with INCLUDE (resource_id, last_updated)   17 MB
/// without                                  8200 kB
/// idx_search_string, same rows             8200 kB
/// ```
///
/// 2.1x, and without it the index lands exactly at its sibling's size. On the
/// 22.6M-row reproduction the seek is unaffected: 31 vs 25 buffers on the narrow
/// shape, 3 vs 4 on the zero-match shape, 719 vs 733 on the whole-slice shape —
/// the difference is heap fetches the plan was already paying. The run measured
/// this index at 467 MB against `idx_search_string`'s 47 MB for the same
/// 205,918 rows, so most of that 467 MB is churn bloat rather than payload; but
/// bloat is proportional to the pages there are to bloat, and this halves them.
///
/// Rolling deploys are safe in both directions. An old binary's SQL —
/// `value_string IS NOT NULL AND COALESCE(…) LIKE $n` — still proves the new
/// predicate, via the strict `~~`, so it keeps working against the new index
/// exactly as it does today (verified: same plan, same 1,709 buffers). A new
/// binary against an un-migrated database is not reachable: migrations complete
/// under the advisory lock before the instance serves.
async fn migrate_v32_to_v33(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let stmts = [
        // Same key columns and same population as v24 built. Two things
        // change: the predicate is reworded onto the expression the query
        // actually compares, so a strict operator on it — `~>=~`, `~<~`, `~~` —
        // proves the index usable with no help from a separate conjunct; and
        // v24's `INCLUDE (resource_id, last_updated)` goes, because an
        // index-only scan over this index is impossible by construction (see
        // the docstring) and the payload's real cost is that it disables btree
        // deduplication.
        "DROP INDEX IF EXISTS idx_search_string_folded_pattern",
        "CREATE INDEX IF NOT EXISTS idx_search_string_folded_pattern
         ON search_index (tenant_id, resource_type, param_name,
                          (COALESCE(value_string_folded, lower(value_string))) text_pattern_ops)
         WHERE COALESCE(value_string_folded, lower(value_string)) IS NOT NULL",
    ];

    for sql in stmts {
        client
            .execute(sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v32->v33 failed: {}", e)))?;
    }

    // The rebuilt index carries the statistics for its expression, and the
    // planner needs them for the new range predicate. Best-effort: a failure
    // here costs plan quality, not correctness.
    if let Err(e) = client.execute("ANALYZE search_index", &[]).await {
        tracing::warn!(
            "Migration v32->v33: optional ANALYZE failed (plans may be suboptimal \
             until autovacuum catches up, search remains correct): {}",
            e
        );
    }

    Ok(())
}

/// v33 -> v34: stop maintaining three indexes nothing reads, and compress the
/// two JSONB columns with LZ4 instead of PGLZ.
///
/// Every council round so far has attacked `search_index`. This one attacks the
/// other 62% of the crud suite. On run 33128380492 the `resources` /
/// `resource_history` group cost 4,773.5 s of the suite's 12,856 s of Postgres
/// execution time:
///
/// ```text
/// exec_s   calls    mean_ms   statement
/// 1664.9   275382     6.046   WITH ins AS (INSERT INTO resources … ) INSERT INTO resource_history …
/// 1437.4   259914     5.530   WITH upd AS (UPDATE  resources … ) INSERT INTO resource_history …
///  963.5   275382     3.499   UPDATE resources SET is_deleted = …, deleted_at = …
///  707.7   275382     2.570   INSERT INTO resource_history (…)
/// ```
///
/// Those four statements write 810,678 `resources` tuples and 810,678
/// `resource_history` tuples in a 302-second run. Postgres executes 12,856 s of
/// work in that window — roughly 43x concurrency against a 4-CPU container — so
/// the database is saturated and work removed converts close to linearly into
/// throughput. Both halves of this migration remove work from the tuple-write
/// path itself, which is the part of those statements no query plan can avoid.
///
/// ## Three indexes that cannot serve a query
///
/// Tracing the whole chain forward rather than reading `create_indexes` — v1
/// builds two on each table, v7 adds `idx_resources_fhir_version`, v15 adds the
/// partial `idx_resources_search`, and nothing before this migration drops any
/// of them — a `resources` write maintains **five** indexes and a
/// `resource_history` write **three**. Three of those eight can never be chosen
/// over an index that is already there:
///
/// | index | why it is dead |
/// |---|---|
/// | `idx_resources_type (tenant_id, resource_type)` | a strict **prefix** of `resources_pkey (tenant_id, resource_type, id)` |
/// | `idx_history_resource (tenant_id, resource_type, id)` | a strict **prefix** of `resource_history_pkey (tenant_id, resource_type, id, version_id)` |
/// | `idx_resources_fhir_version (tenant_id, fhir_version)` | no statement in the workspace filters, joins, groups or orders by `fhir_version` |
///
/// A prefix index answers no predicate its table's primary key does not already
/// answer, in the same order, with the same leading columns — and in
/// `idx_resources_type`'s case v15's `idx_resources_search` leads with the same
/// two columns as well. The `fhir_version` index is a stronger case still:
/// `grep` over every `.rs` and `.sql` file in `crates/` finds `fhir_version`
/// only ever *selected*, never constrained. It has been maintained on every
/// write since schema v7 and never once been read.
///
/// That takes `resources` from five indexes to three and `resource_history` from
/// three to two — 810,678 fewer index-entry insertions on each table per crud
/// run, 1.6M in total, plus the heap and WAL those entries occupy. Note the
/// soft-delete `UPDATE` is not a HOT update and never can be, because it moves
/// `last_updated`, which `idx_resources_updated` indexes; it therefore touches
/// *every* index on the table, so removing two of its five is a direct cut to
/// its index work.
///
/// The two remaining `resources` indexes are both load-bearing and stay:
/// `idx_resources_updated` serves a `_lastUpdated` search with no resource type
/// bound, which v15's type-leading index cannot, and `idx_resources_search`
/// serves every type-scoped search's `ORDER BY last_updated DESC, id ASC`.
///
/// What is given up is small and named: a `SELECT COUNT(*) … WHERE tenant_id = $1
/// AND resource_type = $2` that used to index-only-scan the narrow
/// `idx_resources_type` now scans `resources_pkey`, which carries the `id`
/// column as well and is therefore a few times wider per entry. That is a scan
/// over a warm b-tree on the `_total=accurate` path, traded against 1.6M writes.
///
/// Dropping `idx_resources_type` also removes a live planner footgun. The
/// backend runs prepared statements, so `read`, `update` and `delete` execute
/// under a **generic** plan, and their predicate is
/// `tenant_id = $1 AND resource_type = $2 AND id = $3`. With this index present
/// the planner has two candidates that both cost out at `rows=1`, and in a
/// tenant dominated by one resource type it picks the prefix index and filters
/// on `id`. That was reproduced locally on Postgres 18: the point read planned
/// as `Index Scan using idx_resources_type … Filter: (id = $1), Rows Removed by
/// Filter: 4235, Buffers: shared hit=1575` — 783 buffer hits for a primary-key
/// lookup that needs three. Removing the index removes the choice.
///
/// ## PGLZ -> LZ4 on `resources.data` and `resource_history.data`
///
/// A stored FHIR resource is a few kilobytes of JSON. Once the tuple exceeds
/// `TOAST_TUPLE_THRESHOLD` the toaster compresses `data`, and with the default
/// `default_toast_compression = pglz` that compression runs in the Postgres
/// backend on **every create, every update, and every history row** — three
/// compressions per crud cycle, 1.6M per run. PGLZ manages roughly 50-100 MB/s;
/// LZ4 is several times that for a slightly worse ratio, and Postgres has
/// supported it as a per-column TOAST method since 14. The benchmark's
/// `postgres:18` image ships with LZ4 built in.
///
/// The ratio cost is small at this shape: over the 60,000-resource local corpus
/// the mean compressed `data` datum is 1,532 bytes under PGLZ and 1,586 under
/// LZ4 — 3.5% larger — and both stay inline, so no extra TOAST chunks appear.
/// Decompression gets faster too (0.0216 ms to 0.0185 ms for a point read that
/// returns `data`), which the 1,101,528 reads in a crud run also collect.
///
/// ## Measured
///
/// Local Postgres 18.6, a 200,000-row pre-populated `resources` spread over 140
/// resource types (so the planner sees realistic selectivity), `ANALYZE`d,
/// carrying the full live index set including v15's `idx_resources_search`, and
/// driven by pgbench with one client through the exact statement shapes this
/// backend issues — create CTE, update CTE, pre-delete read, soft delete,
/// history insert. The metric is summed Postgres execution time per crud cycle.
/// Seven alternating repetitions per arm; medians:
///
/// ```text
/// arm                                          ms/cycle   vs baseline
/// current (8 indexes, pglz, split delete)        0.4264         —
/// drop the three indexes only                    0.3520      -17.4%
/// lz4 only                                       0.3513      -17.6%
/// both (this migration)                          0.2734      -35.9%
/// both + the folded delete in storage.rs         0.2285      -46.4%
/// ```
///
/// Applied to the 4,773.5 s those four statements cost on run 33128380492, -36%
/// is about **1,700 s removed from a 12,856 s suite, 13% of crud**, and the
/// storage.rs change on top of it a few hundred more. That is an expectation,
/// not a promise. The lab is a single client on a 200,000-row corpus with
/// `fsync=off`, so it measures backend CPU and buffer work and nothing of the
/// lock contention or I/O that 300 virtual users against a 4-CPU container with
/// 1.6M resources will add. Index depth and cache pressure are both understated,
/// which if anything understates the index half; the LZ4 half is pure backend
/// CPU and should carry over most directly.
///
/// ## What this costs a real database
///
/// Three `DROP INDEX`es and four catalog-only `ALTER TABLE … SET COMPRESSION`.
/// Nothing is rewritten. `SET COMPRESSION` applies to values written from now
/// on; rows already stored keep their PGLZ datums and are read back exactly as
/// before, because the compression method travels in the varlena header. There
/// is therefore no backfill to run and no window in which the table is
/// inconsistent.
///
/// The `ALTER`s are attempted individually and a failure is logged rather than
/// propagated: a Postgres built without `--with-lz4` rejects the method, and
/// that is a reason to keep PGLZ, not a reason to refuse to start.
///
/// A note for whoever picks this up next: `default_toast_compression` can be set
/// per database (`ALTER DATABASE … SET default_toast_compression = 'lz4'`),
/// which would extend the same win to `search_index`'s text columns. That table
/// belongs to another seat and was not measured here, so this migration stays
/// column-scoped.
async fn migrate_v33_to_v34(client: &deadpool_postgres::Client) -> StorageResult<()> {
    let stmts = [
        // 1-2. Strict prefixes of their tables' primary keys.
        "DROP INDEX IF EXISTS idx_resources_type",
        "DROP INDEX IF EXISTS idx_history_resource",
        // 3. Maintained since v7; no statement in the workspace constrains
        //    `fhir_version`.
        "DROP INDEX IF EXISTS idx_resources_fhir_version",
    ];

    for sql in stmts {
        client
            .execute(sql, &[])
            .await
            .map_err(|e| pg_error(format!("Migration v33->v34 failed: {}", e)))?;
    }

    // Catalog-only, and optional: a server built without LZ4 rejects the method
    // and keeps PGLZ, which is correct, just slower. The tsvector columns are
    // included because a large narrative pushes `resource_fts` over the TOAST
    // threshold on exactly the write path this migration is trying to cheapen.
    let compression = [
        "ALTER TABLE resources ALTER COLUMN data SET COMPRESSION lz4",
        "ALTER TABLE resource_history ALTER COLUMN data SET COMPRESSION lz4",
        "ALTER TABLE resource_fts ALTER COLUMN narrative_tsvector SET COMPRESSION lz4",
        "ALTER TABLE resource_fts ALTER COLUMN content_tsvector SET COMPRESSION lz4",
    ];

    for sql in compression {
        if let Err(e) = client.execute(sql, &[]).await {
            tracing::warn!(
                "LZ4 TOAST compression is unavailable on this server, keeping PGLZ ({sql}): {e}"
            );
        }
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
        //
        // SUPERSEDED BY v24, which appends `last_updated DESC, resource_id ASC`
        // as key columns. As written here the index has neither payload nor sort
        // key, so the `system|code` form — the top statement of the search suite
        // — heap-fetched every matching row and sorted the whole match set.
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
        //
        // SUPERSEDED BY v24. As written here the index is unreachable: it is
        // partial on `value_string IS NOT NULL`, and `COALESCE(a, b) LIKE $n`
        // does not imply that (COALESCE is not strict), so Postgres may not use
        // it — 0 scans in run 33029355759 while the whole string load fell to
        // `idx_search_string_folded`. v24 rebuilds it with a covering payload,
        // and `build_string_condition` now emits the conjunct that makes the
        // predicate provable.
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
