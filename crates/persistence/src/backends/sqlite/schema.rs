//! SQLite schema definitions and migrations.

use rusqlite::Connection;

use crate::error::StorageResult;

/// Current schema version.
pub const SCHEMA_VERSION: i32 = 19;

/// Initialize the database schema.
pub fn initialize_schema(conn: &Connection) -> StorageResult<()> {
    // Check current version
    let current_version = get_schema_version(conn)?;

    if current_version == 0 {
        // Fresh database - create base schema then run all migrations
        create_schema_v1(conn)?;
        set_schema_version(conn, 1)?;
        // Run migrations from v1 to latest
        migrate_schema(conn, 1)?;
    } else if current_version < SCHEMA_VERSION {
        // Run migrations
        migrate_schema(conn, current_version)?;
    }

    // Safety net for the tenant registry (schema v14). A pre-release build could
    // stamp a database at the registry's version without creating the `tenants`
    // table (the migration was completed after the version bump), leaving the
    // version-gated migration above unable to re-run. The table's DDL is
    // `IF NOT EXISTS` and idempotent, so ensuring it here every startup
    // self-heals such databases and is a no-op for correctly-migrated ones.
    ensure_tenants_table(conn)?;

    Ok(())
}

/// Idempotently ensures the tenant-registry table exists. Called on every
/// startup as a self-heal (see [`initialize_schema`]); also the body of the
/// v12 -> v13 migration.
fn ensure_tenants_table(conn: &Connection) -> StorageResult<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS tenants (
            id TEXT PRIMARY KEY,
            display_name TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
        [],
    )
    .map_err(|e| migration_err(format!("ensure tenants table: {e}")))?;
    Ok(())
}

/// Get the current schema version.
fn get_schema_version(conn: &Connection) -> StorageResult<i32> {
    // Create version table if it doesn't exist
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER NOT NULL
        )",
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create schema_version table: {}", e),
            source: None,
        })
    })?;

    let version: Option<i32> = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
            row.get(0)
        })
        .ok();

    Ok(version.unwrap_or(0))
}

/// Set the schema version.
fn set_schema_version(conn: &Connection, version: i32) -> StorageResult<()> {
    conn.execute("DELETE FROM schema_version", [])
        .map_err(|e| {
            crate::error::StorageError::Backend(crate::error::BackendError::Internal {
                backend_name: "sqlite".to_string(),
                message: format!("Failed to clear schema_version: {}", e),
                source: None,
            })
        })?;

    conn.execute(
        "INSERT INTO schema_version (version) VALUES (?1)",
        [version],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to set schema_version: {}", e),
            source: None,
        })
    })?;

    Ok(())
}

/// Create the initial schema (version 1).
fn create_schema_v1(conn: &Connection) -> StorageResult<()> {
    // Main resources table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS resources (
            tenant_id TEXT NOT NULL,
            resource_type TEXT NOT NULL,
            id TEXT NOT NULL,
            version_id TEXT NOT NULL,
            data BLOB NOT NULL,
            last_updated TEXT NOT NULL,
            is_deleted INTEGER NOT NULL DEFAULT 0,
            deleted_at TEXT,
            PRIMARY KEY (tenant_id, resource_type, id)
        )",
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create resources table: {}", e),
            source: None,
        })
    })?;

    // Resource history table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS resource_history (
            tenant_id TEXT NOT NULL,
            resource_type TEXT NOT NULL,
            id TEXT NOT NULL,
            version_id TEXT NOT NULL,
            data BLOB NOT NULL,
            last_updated TEXT NOT NULL,
            is_deleted INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (tenant_id, resource_type, id, version_id)
        )",
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create resource_history table: {}", e),
            source: None,
        })
    })?;

    // Search index table for extracted values
    conn.execute(
        "CREATE TABLE IF NOT EXISTS search_index (
            tenant_id TEXT NOT NULL,
            resource_type TEXT NOT NULL,
            resource_id TEXT NOT NULL,
            param_name TEXT NOT NULL,
            param_url TEXT,
            value_string TEXT,
            value_token_system TEXT,
            value_token_code TEXT,
            value_token_display TEXT,
            value_date TEXT,
            value_date_precision TEXT,
            value_number REAL,
            value_quantity_value REAL,
            value_quantity_unit TEXT,
            value_quantity_system TEXT,
            value_reference TEXT,
            value_uri TEXT,
            composite_group INTEGER,
            value_identifier_type_system TEXT,
            value_identifier_type_code TEXT,
            value_reference_display TEXT,
            FOREIGN KEY (tenant_id, resource_type, resource_id)
                REFERENCES resources(tenant_id, resource_type, id) ON DELETE CASCADE
        )",
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create search_index table: {}", e),
            source: None,
        })
    })?;

    // Create indexes for efficient queries
    create_indexes(conn)?;

    // Create FTS5 table for full-text search (if available)
    create_fts_table(conn)?;

    Ok(())
}

/// Create indexes for efficient queries.
fn create_indexes(conn: &Connection) -> StorageResult<()> {
    let indexes = [
        // Resources table indexes
        "CREATE INDEX IF NOT EXISTS idx_resources_type ON resources(tenant_id, resource_type)",
        "CREATE INDEX IF NOT EXISTS idx_resources_updated ON resources(tenant_id, last_updated)",
        "CREATE INDEX IF NOT EXISTS idx_resources_reindex ON resources(tenant_id, resource_type, last_updated, id)",
        // History table indexes
        "CREATE INDEX IF NOT EXISTS idx_history_resource ON resource_history(tenant_id, resource_type, id)",
        "CREATE INDEX IF NOT EXISTS idx_history_updated ON resource_history(tenant_id, last_updated)",
        // Search index indexes
        "CREATE INDEX IF NOT EXISTS idx_search_string ON search_index(tenant_id, resource_type, param_name, value_string) WHERE value_string IS NOT NULL",
        "CREATE INDEX IF NOT EXISTS idx_search_token ON search_index(tenant_id, resource_type, param_name, value_token_system, value_token_code) WHERE value_token_system IS NOT NULL OR value_token_code IS NOT NULL",
        "CREATE INDEX IF NOT EXISTS idx_search_date ON search_index(tenant_id, resource_type, param_name, value_date) WHERE value_date IS NOT NULL",
        "CREATE INDEX IF NOT EXISTS idx_search_number ON search_index(tenant_id, resource_type, param_name, value_number) WHERE value_number IS NOT NULL",
        "CREATE INDEX IF NOT EXISTS idx_search_quantity ON search_index(tenant_id, resource_type, param_name, value_quantity_value, value_quantity_unit) WHERE value_quantity_value IS NOT NULL",
        "CREATE INDEX IF NOT EXISTS idx_search_reference ON search_index(tenant_id, resource_type, param_name, value_reference) WHERE value_reference IS NOT NULL",
        "CREATE INDEX IF NOT EXISTS idx_search_uri ON search_index(tenant_id, resource_type, param_name, value_uri) WHERE value_uri IS NOT NULL",
        // Index for composite parameter matching
        "CREATE INDEX IF NOT EXISTS idx_search_composite ON search_index(tenant_id, resource_type, resource_id, param_name, composite_group)",
        // Index for resource-based lookups
        "CREATE INDEX IF NOT EXISTS idx_search_resource ON search_index(tenant_id, resource_type, resource_id)",
        // Index for :text modifier searches (token display text)
        "CREATE INDEX IF NOT EXISTS idx_search_token_display ON search_index(tenant_id, resource_type, param_name, value_token_display) WHERE value_token_display IS NOT NULL",
        // Index for :of-type modifier searches (identifier type)
        "CREATE INDEX IF NOT EXISTS idx_search_identifier_type ON search_index(tenant_id, resource_type, param_name, value_identifier_type_system, value_identifier_type_code) WHERE value_identifier_type_system IS NOT NULL OR value_identifier_type_code IS NOT NULL",
    ];

    for index_sql in &indexes {
        conn.execute(index_sql, []).map_err(|e| {
            crate::error::StorageError::Backend(crate::error::BackendError::Internal {
                backend_name: "sqlite".to_string(),
                message: format!("Failed to create index: {}", e),
                source: None,
            })
        })?;
    }

    Ok(())
}

/// Create FTS5 virtual table for full-text search.
///
/// This is optional - if FTS5 is not available, the function succeeds silently.
fn create_fts_table(conn: &Connection) -> StorageResult<()> {
    // Check if FTS5 is available
    let fts5_available: i32 = conn
        .query_row(
            "SELECT sqlite_compileoption_used('ENABLE_FTS5')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if fts5_available == 0 {
        // FTS5 not available - skip silently
        return Ok(());
    }

    // Create the FTS5 virtual table for full-text search
    conn.execute(
        "CREATE VIRTUAL TABLE IF NOT EXISTS resource_fts USING fts5(
            resource_id UNINDEXED,
            resource_type UNINDEXED,
            tenant_id UNINDEXED,
            narrative_text,
            full_content,
            tokenize='porter unicode61 remove_diacritics 1'
        )",
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create resource_fts table: {}", e),
            source: None,
        })
    })?;

    Ok(())
}

/// Run schema migrations from current version to latest.
fn migrate_schema(conn: &Connection, from_version: i32) -> StorageResult<()> {
    let mut version = from_version;

    while version < SCHEMA_VERSION {
        match version {
            1 => migrate_v1_to_v2(conn)?,
            2 => migrate_v2_to_v3(conn)?,
            3 => migrate_v3_to_v4(conn)?,
            4 => migrate_v4_to_v5(conn)?,
            5 => migrate_v5_to_v6(conn)?,
            6 => migrate_v6_to_v7(conn)?,
            7 => migrate_v7_to_v8(conn)?,
            8 => migrate_v8_to_v9(conn)?,
            9 => migrate_v9_to_v10(conn)?,
            10 => migrate_v10_to_v11(conn)?,
            11 => migrate_v11_to_v12(conn)?,
            12 => migrate_v12_to_v13(conn)?,
            13 => migrate_v13_to_v14(conn)?,
            14 => migrate_v14_to_v15(conn)?,
            15 => migrate_v15_to_v16(conn)?,
            16 => migrate_v16_to_v17(conn)?,
            17 => migrate_v17_to_v18(conn)?,
            18 => migrate_v18_to_v19(conn)?,
            _ => {
                return Err(crate::error::StorageError::Backend(
                    crate::error::BackendError::Internal {
                        backend_name: "sqlite".to_string(),
                        message: format!("Unknown schema version: {}", version),
                        source: None,
                    },
                ));
            }
        }
        version += 1;
        set_schema_version(conn, version)?;
    }

    Ok(())
}

/// Migrate from schema version 1 to version 2.
///
/// This migration adds new columns to the search_index table:
/// - param_url: Canonical URL for the search parameter
/// - value_date_precision: Precision tracking for date values
/// - value_quantity_system: System URI for quantity units
/// - composite_group: Group ID for composite parameter components
fn migrate_v1_to_v2(conn: &Connection) -> StorageResult<()> {
    let migrations = [
        // Add new columns to search_index table
        "ALTER TABLE search_index ADD COLUMN param_url TEXT",
        "ALTER TABLE search_index ADD COLUMN value_date_precision TEXT",
        "ALTER TABLE search_index ADD COLUMN value_quantity_system TEXT",
        "ALTER TABLE search_index ADD COLUMN composite_group INTEGER",
    ];

    for sql in &migrations {
        // Ignore errors for column already exists (idempotent migration)
        let _ = conn.execute(sql, []);
    }

    // Create new indexes
    let indexes = [
        "CREATE INDEX IF NOT EXISTS idx_search_quantity ON search_index(tenant_id, resource_type, param_name, value_quantity_value, value_quantity_unit)",
        "CREATE INDEX IF NOT EXISTS idx_search_composite ON search_index(tenant_id, resource_type, resource_id, param_name, composite_group)",
        "CREATE INDEX IF NOT EXISTS idx_search_resource ON search_index(tenant_id, resource_type, resource_id)",
    ];

    for index_sql in &indexes {
        conn.execute(index_sql, []).map_err(|e| {
            crate::error::StorageError::Backend(crate::error::BackendError::Internal {
                backend_name: "sqlite".to_string(),
                message: format!("Failed to create index in migration: {}", e),
                source: None,
            })
        })?;
    }

    Ok(())
}

/// Migrate from schema version 2 to version 3.
///
/// This migration adds FTS5 full-text search support for _text and _content searches:
/// - resource_fts: FTS5 virtual table for full-text search
/// - Stores narrative text (for _text) and full content (for _content)
fn migrate_v2_to_v3(conn: &Connection) -> StorageResult<()> {
    // Check if FTS5 is available
    let fts5_available: i32 = conn
        .query_row(
            "SELECT sqlite_compileoption_used('ENABLE_FTS5')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if fts5_available == 0 {
        // FTS5 not available - log warning but don't fail
        // _text and _content searches will be unsupported
        tracing::warn!("FTS5 not available - full-text search features will be disabled");
        return Ok(());
    }

    // Create the FTS5 virtual table for full-text search.
    //
    // NOTE: this is a plain fts5 table, NOT external-content mode — there is no
    // `content=` clause. It therefore keeps a full second copy of every
    // resource's narrative and serialized body in its `resource_fts_content`
    // shadow table. That matters when reasoning about where purged PHI lives:
    // deleting the `resources` row does not reach this copy (see issue #386).
    conn.execute(
        "CREATE VIRTUAL TABLE IF NOT EXISTS resource_fts USING fts5(
            resource_id UNINDEXED,
            resource_type UNINDEXED,
            tenant_id UNINDEXED,
            narrative_text,
            full_content,
            tokenize='porter unicode61 remove_diacritics 1'
        )",
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create resource_fts table: {}", e),
            source: None,
        })
    })?;

    Ok(())
}

/// Migrate from schema version 3 to version 4.
///
/// This migration adds columns for enhanced token search:
/// - value_token_display: Display text for Coding.display and CodeableConcept.text (:text modifier)
/// - value_identifier_type_system: System URI for Identifier.type.coding (:of-type modifier)
/// - value_identifier_type_code: Code for Identifier.type.coding (:of-type modifier)
fn migrate_v3_to_v4(conn: &Connection) -> StorageResult<()> {
    let migrations = [
        // Add columns for :text modifier support (token display text)
        "ALTER TABLE search_index ADD COLUMN value_token_display TEXT",
        // Add columns for :of-type modifier support (identifier type)
        "ALTER TABLE search_index ADD COLUMN value_identifier_type_system TEXT",
        "ALTER TABLE search_index ADD COLUMN value_identifier_type_code TEXT",
    ];

    for sql in &migrations {
        // Ignore errors for column already exists (idempotent migration)
        let _ = conn.execute(sql, []);
    }

    // Create indexes for efficient searching
    let indexes = [
        // Index for :text modifier searches
        "CREATE INDEX IF NOT EXISTS idx_search_token_display ON search_index(tenant_id, resource_type, param_name, value_token_display)",
        // Index for :of-type modifier searches
        "CREATE INDEX IF NOT EXISTS idx_search_identifier_type ON search_index(tenant_id, resource_type, param_name, value_identifier_type_system, value_identifier_type_code)",
    ];

    for index_sql in &indexes {
        conn.execute(index_sql, []).map_err(|e| {
            crate::error::StorageError::Backend(crate::error::BackendError::Internal {
                backend_name: "sqlite".to_string(),
                message: format!("Failed to create index in migration: {}", e),
                source: None,
            })
        })?;
    }

    Ok(())
}

/// Migrate from schema version 4 to version 5.
///
/// This migration updates FTS5 triggers to also index token display text
/// (value_token_display), enabling the :text-advanced modifier to search
/// on Coding.display and CodeableConcept.text fields.
fn migrate_v4_to_v5(conn: &Connection) -> StorageResult<()> {
    // Check if FTS5 is available
    let fts5_available: i32 = conn
        .query_row(
            "SELECT sqlite_compileoption_used('ENABLE_FTS5')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if fts5_available == 0 {
        // FTS5 not available - skip
        tracing::warn!("FTS5 not available - :text-advanced modifier will not work");
        return Ok(());
    }

    // Check if search_index_fts table exists
    let fts_exists: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='search_index_fts'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if fts_exists == 0 {
        // FTS table doesn't exist - create it with updated schema
        conn.execute(
            r#"
            CREATE VIRTUAL TABLE IF NOT EXISTS search_index_fts USING fts5(
                text_content,
                content='search_index',
                content_rowid='rowid',
                tokenize='porter unicode61'
            )
            "#,
            [],
        )
        .map_err(|e| {
            crate::error::StorageError::Backend(crate::error::BackendError::Internal {
                backend_name: "sqlite".to_string(),
                message: format!("Failed to create search_index_fts table: {}", e),
                source: None,
            })
        })?;
    }

    // Drop existing triggers
    let _ = conn.execute("DROP TRIGGER IF EXISTS search_index_fts_insert", []);
    let _ = conn.execute("DROP TRIGGER IF EXISTS search_index_fts_delete", []);
    let _ = conn.execute("DROP TRIGGER IF EXISTS search_index_fts_update", []);

    // Create updated triggers that index both value_string and value_token_display
    conn.execute(
        r#"
        CREATE TRIGGER search_index_fts_insert AFTER INSERT ON search_index
        WHEN new.value_string IS NOT NULL OR new.value_token_display IS NOT NULL
        BEGIN
            INSERT INTO search_index_fts(rowid, text_content)
            VALUES (new.rowid, COALESCE(new.value_string, '') || ' ' || COALESCE(new.value_token_display, ''));
        END
        "#,
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create FTS insert trigger: {}", e),
            source: None,
        })
    })?;

    conn.execute(
        r#"
        CREATE TRIGGER search_index_fts_delete AFTER DELETE ON search_index
        WHEN old.value_string IS NOT NULL OR old.value_token_display IS NOT NULL
        BEGIN
            INSERT INTO search_index_fts(search_index_fts, rowid, text_content)
            VALUES ('delete', old.rowid, COALESCE(old.value_string, '') || ' ' || COALESCE(old.value_token_display, ''));
        END
        "#,
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create FTS delete trigger: {}", e),
            source: None,
        })
    })?;

    conn.execute(
        r#"
        CREATE TRIGGER search_index_fts_update AFTER UPDATE ON search_index
        WHEN old.value_string IS NOT NULL OR new.value_string IS NOT NULL
             OR old.value_token_display IS NOT NULL OR new.value_token_display IS NOT NULL
        BEGIN
            INSERT INTO search_index_fts(search_index_fts, rowid, text_content)
            VALUES ('delete', old.rowid, COALESCE(old.value_string, '') || ' ' || COALESCE(old.value_token_display, ''));
            INSERT INTO search_index_fts(rowid, text_content)
            VALUES (new.rowid, COALESCE(new.value_string, '') || ' ' || COALESCE(new.value_token_display, ''));
        END
        "#,
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create FTS update trigger: {}", e),
            source: None,
        })
    })?;

    // Rebuild the FTS index to include existing token display values
    // This is a one-time operation during migration
    let _ = conn.execute(
        "INSERT INTO search_index_fts(search_index_fts) VALUES ('rebuild')",
        [],
    );

    Ok(())
}

/// Migrate from schema version 5 to version 6.
///
/// This migration adds tables for bulk data export and bulk submit operations:
///
/// Bulk Export tables:
/// - bulk_export_jobs: Export job metadata and status
/// - bulk_export_progress: Per-type progress tracking
/// - bulk_export_files: Output file information
///
/// Bulk Submit tables:
/// - bulk_submissions: Submission metadata and status
/// - bulk_manifests: Manifest metadata within submissions
/// - bulk_entry_results: Per-entry processing results
/// - bulk_submission_changes: Change tracking for rollback
fn migrate_v5_to_v6(conn: &Connection) -> StorageResult<()> {
    // Bulk Export tables
    conn.execute(
        "CREATE TABLE IF NOT EXISTS bulk_export_jobs (
            id TEXT PRIMARY KEY,
            tenant_id TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'accepted',
            level TEXT NOT NULL,
            group_id TEXT,
            request_json TEXT NOT NULL,
            transaction_time TEXT NOT NULL,
            started_at TEXT,
            completed_at TEXT,
            error_message TEXT,
            current_type TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create bulk_export_jobs table: {}", e),
            source: None,
        })
    })?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_export_jobs_tenant
         ON bulk_export_jobs(tenant_id, status)",
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create idx_export_jobs_tenant: {}", e),
            source: None,
        })
    })?;

    conn.execute(
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
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create bulk_export_progress table: {}", e),
            source: None,
        })
    })?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS bulk_export_files (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id TEXT NOT NULL,
            resource_type TEXT NOT NULL,
            file_type TEXT NOT NULL DEFAULT 'output',
            file_path TEXT NOT NULL,
            resource_count INTEGER DEFAULT 0,
            byte_count INTEGER DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            FOREIGN KEY (job_id) REFERENCES bulk_export_jobs(id) ON DELETE CASCADE
        )",
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create bulk_export_files table: {}", e),
            source: None,
        })
    })?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_export_files_job
         ON bulk_export_files(job_id)",
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create idx_export_files_job: {}", e),
            source: None,
        })
    })?;

    // Bulk Submit tables
    conn.execute(
        "CREATE TABLE IF NOT EXISTS bulk_submissions (
            tenant_id TEXT NOT NULL,
            submitter TEXT NOT NULL,
            submission_id TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'in-progress',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            completed_at TEXT,
            metadata BLOB,
            PRIMARY KEY (tenant_id, submitter, submission_id)
        )",
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create bulk_submissions table: {}", e),
            source: None,
        })
    })?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_bulk_submissions_status
         ON bulk_submissions(tenant_id, status)",
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create idx_bulk_submissions_status: {}", e),
            source: None,
        })
    })?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS bulk_manifests (
            tenant_id TEXT NOT NULL,
            submitter TEXT NOT NULL,
            submission_id TEXT NOT NULL,
            manifest_id TEXT NOT NULL,
            manifest_url TEXT,
            replaces_manifest_url TEXT,
            status TEXT NOT NULL DEFAULT 'pending',
            added_at TEXT NOT NULL,
            total_entries INTEGER DEFAULT 0,
            processed_entries INTEGER DEFAULT 0,
            failed_entries INTEGER DEFAULT 0,
            PRIMARY KEY (tenant_id, submitter, submission_id, manifest_id),
            FOREIGN KEY (tenant_id, submitter, submission_id)
                REFERENCES bulk_submissions(tenant_id, submitter, submission_id) ON DELETE CASCADE
        )",
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create bulk_manifests table: {}", e),
            source: None,
        })
    })?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS bulk_entry_results (
            tenant_id TEXT NOT NULL,
            submitter TEXT NOT NULL,
            submission_id TEXT NOT NULL,
            manifest_id TEXT NOT NULL,
            file_url TEXT NOT NULL DEFAULT '',
            line_number INTEGER NOT NULL,
            resource_type TEXT NOT NULL,
            resource_id TEXT,
            created INTEGER,
            outcome TEXT NOT NULL,
            operation_outcome BLOB,
            PRIMARY KEY (tenant_id, submitter, submission_id, manifest_id, file_url, line_number),
            FOREIGN KEY (tenant_id, submitter, submission_id, manifest_id)
                REFERENCES bulk_manifests(tenant_id, submitter, submission_id, manifest_id) ON DELETE CASCADE
        )",
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create bulk_entry_results table: {}", e),
            source: None,
        })
    })?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_bulk_entry_results_outcome
         ON bulk_entry_results(tenant_id, submitter, submission_id, manifest_id, outcome)",
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create idx_bulk_entry_results_outcome: {}", e),
            source: None,
        })
    })?;

    conn.execute(
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
            previous_content BLOB,
            changed_at TEXT NOT NULL,
            PRIMARY KEY (tenant_id, submitter, submission_id, change_id),
            FOREIGN KEY (tenant_id, submitter, submission_id)
                REFERENCES bulk_submissions(tenant_id, submitter, submission_id) ON DELETE CASCADE
        )",
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create bulk_submission_changes table: {}", e),
            source: None,
        })
    })?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_bulk_changes_resource
         ON bulk_submission_changes(tenant_id, resource_type, resource_id)",
        [],
    )
    .map_err(|e| {
        crate::error::StorageError::Backend(crate::error::BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to create idx_bulk_changes_resource: {}", e),
            source: None,
        })
    })?;

    Ok(())
}

/// Migrate from schema version 6 to version 7.
///
/// This migration adds FHIR version tracking to resources:
/// - fhir_version column to resources table (defaults to '4.0' for R4)
/// - fhir_version column to resource_history table (defaults to '4.0' for R4)
/// - Index on fhir_version for efficient version-based queries
fn migrate_v6_to_v7(conn: &Connection) -> StorageResult<()> {
    let migrations = [
        // Add fhir_version column to resources table (default to R4 for existing resources)
        "ALTER TABLE resources ADD COLUMN fhir_version TEXT NOT NULL DEFAULT '4.0'",
        // Add fhir_version column to resource_history table
        "ALTER TABLE resource_history ADD COLUMN fhir_version TEXT NOT NULL DEFAULT '4.0'",
    ];

    for sql in &migrations {
        // Ignore errors for column already exists (idempotent migration)
        let _ = conn.execute(sql, []);
    }

    // Create index for efficient version-based queries
    let indexes = [
        "CREATE INDEX IF NOT EXISTS idx_resources_fhir_version ON resources(tenant_id, fhir_version)",
    ];

    for index_sql in &indexes {
        conn.execute(index_sql, []).map_err(|e| {
            crate::error::StorageError::Backend(crate::error::BackendError::Internal {
                backend_name: "sqlite".to_string(),
                message: format!("Failed to create index in migration: {}", e),
                source: None,
            })
        })?;
    }

    Ok(())
}

/// Migrate from schema version 7 to version 8.
///
/// Adds bulk-export worker/lease support:
/// - lease columns + `owner_subject`/`request_url`/`fhir_version` on `bulk_export_jobs`
/// - `part_index`/`fencing_token` on `bulk_export_files`, with a backfill of
///   `part_index` and a unique index for idempotent upserts
fn migrate_v7_to_v8(conn: &Connection) -> StorageResult<()> {
    // Columns that may already exist if the table was created fresh — guard
    // with PRAGMA table_info since SQLite has no `ADD COLUMN IF NOT EXISTS`.
    let job_columns: Vec<String> = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(bulk_export_jobs)")
            .map_err(|e| migration_err(format!("pragma bulk_export_jobs: {e}")))?;
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| migration_err(format!("pragma rows: {e}")))?
            .filter_map(|r| r.ok())
            .collect();
        cols
    };
    let job_adds = [
        (
            "worker_id",
            "ALTER TABLE bulk_export_jobs ADD COLUMN worker_id TEXT",
        ),
        (
            "lease_expiry",
            "ALTER TABLE bulk_export_jobs ADD COLUMN lease_expiry TEXT",
        ),
        (
            "fencing_token",
            "ALTER TABLE bulk_export_jobs ADD COLUMN fencing_token INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "heartbeat_at",
            "ALTER TABLE bulk_export_jobs ADD COLUMN heartbeat_at TEXT",
        ),
        (
            "owner_subject",
            "ALTER TABLE bulk_export_jobs ADD COLUMN owner_subject TEXT",
        ),
        (
            "request_url",
            "ALTER TABLE bulk_export_jobs ADD COLUMN request_url TEXT NOT NULL DEFAULT ''",
        ),
        (
            "fhir_version",
            "ALTER TABLE bulk_export_jobs ADD COLUMN fhir_version TEXT NOT NULL DEFAULT '4.0'",
        ),
    ];
    for (col, sql) in &job_adds {
        if !job_columns.iter().any(|c| c == col) {
            conn.execute(sql, [])
                .map_err(|e| migration_err(format!("add {col}: {e}")))?;
        }
    }

    let file_columns: Vec<String> = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(bulk_export_files)")
            .map_err(|e| migration_err(format!("pragma bulk_export_files: {e}")))?;
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| migration_err(format!("pragma rows: {e}")))?
            .filter_map(|r| r.ok())
            .collect();
        cols
    };
    let file_adds = [
        (
            "part_index",
            "ALTER TABLE bulk_export_files ADD COLUMN part_index INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "fencing_token",
            "ALTER TABLE bulk_export_files ADD COLUMN fencing_token INTEGER NOT NULL DEFAULT 0",
        ),
    ];
    for (col, sql) in &file_adds {
        if !file_columns.iter().any(|c| c == col) {
            conn.execute(sql, [])
                .map_err(|e| migration_err(format!("add {col}: {e}")))?;
        }
    }

    // Backfill part_index: 0-based sequential per (job_id, file_type, resource_type)
    // ordered by id, so the unique index below builds without collisions on
    // pre-existing rows.
    conn.execute(
        "UPDATE bulk_export_files SET part_index = (
            SELECT COUNT(*) FROM bulk_export_files f2
            WHERE f2.job_id = bulk_export_files.job_id
              AND f2.file_type = bulk_export_files.file_type
              AND f2.resource_type = bulk_export_files.resource_type
              AND f2.id < bulk_export_files.id
        )",
        [],
    )
    .map_err(|e| migration_err(format!("backfill part_index: {e}")))?;

    let indexes = [
        "CREATE INDEX IF NOT EXISTS idx_export_jobs_claim
         ON bulk_export_jobs(tenant_id, status, lease_expiry)",
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_export_files_part
         ON bulk_export_files(job_id, file_type, resource_type, part_index)",
    ];
    for index_sql in &indexes {
        conn.execute(index_sql, [])
            .map_err(|e| migration_err(format!("create index: {e}")))?;
    }

    Ok(())
}

/// Migrate from schema version 8 to version 9.
///
/// Adds the async Bulk Data Submit worker layer on top of the existing
/// synchronous bulk-submit ingestion tables:
/// - `bulk_submissions`: poll-token / owner / transaction-time / access-token /
///   request-url columns (REST status + auth need these).
/// - `bulk_manifests`: worker lease + fencing columns, the kickoff parameters
///   needed to fetch the remote manifest (fhir base url, output format, request
///   headers, oauth metadata urls, encryption key), and a resume cursor.
/// - `bulk_submit_files`: status-manifest output/error/deleted artifact rows.
fn migrate_v8_to_v9(conn: &Connection) -> StorageResult<()> {
    add_bulk_submit_worker_schema(conn)
}

fn add_bulk_submit_worker_schema(conn: &Connection) -> StorageResult<()> {
    // bulk_submissions: REST status + auth columns.
    let submission_columns: Vec<String> = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(bulk_submissions)")
            .map_err(|e| migration_err(format!("pragma bulk_submissions: {e}")))?;
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| migration_err(format!("pragma rows: {e}")))?
            .filter_map(|r| r.ok())
            .collect();
        cols
    };
    let submission_adds = [
        (
            "owner_subject",
            "ALTER TABLE bulk_submissions ADD COLUMN owner_subject TEXT",
        ),
        (
            "poll_token",
            "ALTER TABLE bulk_submissions ADD COLUMN poll_token TEXT",
        ),
        (
            "transaction_time",
            "ALTER TABLE bulk_submissions ADD COLUMN transaction_time TEXT",
        ),
        (
            "requires_access_token",
            "ALTER TABLE bulk_submissions ADD COLUMN requires_access_token INTEGER",
        ),
        (
            "request_url",
            "ALTER TABLE bulk_submissions ADD COLUMN request_url TEXT",
        ),
    ];
    for (col, sql) in &submission_adds {
        if !submission_columns.iter().any(|c| c == col) {
            conn.execute(sql, [])
                .map_err(|e| migration_err(format!("add {col}: {e}")))?;
        }
    }

    // bulk_manifests: worker lease/fencing + kickoff parameters + resume cursor.
    let manifest_columns: Vec<String> = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(bulk_manifests)")
            .map_err(|e| migration_err(format!("pragma bulk_manifests: {e}")))?;
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| migration_err(format!("pragma rows: {e}")))?
            .filter_map(|r| r.ok())
            .collect();
        cols
    };
    let manifest_adds = [
        (
            "worker_id",
            "ALTER TABLE bulk_manifests ADD COLUMN worker_id TEXT",
        ),
        (
            "lease_expiry",
            "ALTER TABLE bulk_manifests ADD COLUMN lease_expiry TEXT",
        ),
        (
            "fencing_token",
            "ALTER TABLE bulk_manifests ADD COLUMN fencing_token INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "fhir_base_url",
            "ALTER TABLE bulk_manifests ADD COLUMN fhir_base_url TEXT",
        ),
        (
            "output_format",
            "ALTER TABLE bulk_manifests ADD COLUMN output_format TEXT",
        ),
        (
            "file_request_headers",
            "ALTER TABLE bulk_manifests ADD COLUMN file_request_headers TEXT",
        ),
        (
            "oauth_metadata_urls",
            "ALTER TABLE bulk_manifests ADD COLUMN oauth_metadata_urls TEXT",
        ),
        (
            "file_encryption_key",
            "ALTER TABLE bulk_manifests ADD COLUMN file_encryption_key TEXT",
        ),
        (
            "last_processed_line",
            "ALTER TABLE bulk_manifests ADD COLUMN last_processed_line INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "import_directives",
            "ALTER TABLE bulk_manifests ADD COLUMN import_directives TEXT",
        ),
        (
            "submission_metadata",
            "ALTER TABLE bulk_manifests ADD COLUMN submission_metadata TEXT",
        ),
    ];
    for (col, sql) in &manifest_adds {
        if !manifest_columns.iter().any(|c| c == col) {
            conn.execute(sql, [])
                .map_err(|e| migration_err(format!("add {col}: {e}")))?;
        }
    }

    // Status-manifest artifact rows (output/error/deleted NDJSON parts).
    conn.execute(
        "CREATE TABLE IF NOT EXISTS bulk_submit_files (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            tenant_id TEXT NOT NULL,
            submitter TEXT NOT NULL,
            submission_id TEXT NOT NULL,
            manifest_url TEXT,
            file_type TEXT NOT NULL,
            resource_type TEXT,
            part_index INTEGER NOT NULL DEFAULT 0,
            fencing_token INTEGER NOT NULL DEFAULT 0,
            file_path TEXT NOT NULL,
            line_count INTEGER NOT NULL DEFAULT 0,
            byte_count INTEGER NOT NULL DEFAULT 0,
            count_severity TEXT,
            created_at TEXT NOT NULL,
            FOREIGN KEY (tenant_id, submitter, submission_id)
                REFERENCES bulk_submissions(tenant_id, submitter, submission_id) ON DELETE CASCADE
        )",
        [],
    )
    .map_err(|e| migration_err(format!("create bulk_submit_files: {e}")))?;

    let indexes = [
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_bulk_submissions_poll_token
         ON bulk_submissions(poll_token)",
        "CREATE INDEX IF NOT EXISTS idx_bulk_manifests_claim
         ON bulk_manifests(tenant_id, status, lease_expiry)",
        "CREATE INDEX IF NOT EXISTS idx_bulk_submit_files_submission
         ON bulk_submit_files(tenant_id, submitter, submission_id)",
    ];
    for index_sql in &indexes {
        conn.execute(index_sql, [])
            .map_err(|e| migration_err(format!("create index: {e}")))?;
    }

    Ok(())
}

/// Migrate from schema version 9 to version 10.
///
/// Adds:
/// - reference display text for reference modifiers;
/// - UCUM-canonicalized quantity columns so quantity search matches across
///   equivalent units (e.g. `1 g` ⇄ `1000 mg`); and
/// - a case/accent-folded string column so string search is accent-insensitive.
///
/// Existing rows have NULL values in the new columns until a reindex backfills
/// them; the handlers fall back to raw matching for those rows.
fn migrate_v9_to_v10(conn: &Connection) -> StorageResult<()> {
    // SQLite has no `ADD COLUMN IF NOT EXISTS`; ignore duplicate-column errors
    // (the columns may already exist if the table was created fresh at v10).
    let _ = conn.execute(
        "ALTER TABLE search_index ADD COLUMN value_reference_display TEXT",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE search_index ADD COLUMN value_quantity_canonical_value REAL",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE search_index ADD COLUMN value_quantity_canonical_unit TEXT",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE search_index ADD COLUMN value_string_folded TEXT",
        [],
    );
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_search_reference_display
         ON search_index(tenant_id, resource_type, param_name, value_reference_display)",
        [],
    )
    .map_err(|e| migration_err(format!("create reference_display index: {e}")))?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_search_quantity_canonical
         ON search_index(tenant_id, resource_type, param_name, value_quantity_canonical_unit, value_quantity_canonical_value)",
        [],
    )
    .map_err(|e| migration_err(format!("create canonical quantity index: {e}")))?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_search_string_folded
         ON search_index(tenant_id, resource_type, param_name, value_string_folded)",
        [],
    )
    .map_err(|e| migration_err(format!("create folded string index: {e}")))?;
    Ok(())
}

/// Migrate from schema version 10 to version 11.
///
/// Adds columns supporting `_contained` search: index rows extracted from a
/// container's `contained[]` entries are flagged `is_contained = 1` and carry
/// the contained resource's type and local id. The row's `resource_type` /
/// `resource_id` continue to identify the *container* (preserving the FK to
/// `resources`), while `contained_type` records the nested resource's type.
fn migrate_v10_to_v11(conn: &Connection) -> StorageResult<()> {
    // SQLite has no `ADD COLUMN IF NOT EXISTS`; ignore duplicate-column errors.
    let _ = conn.execute(
        "ALTER TABLE search_index ADD COLUMN is_contained INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE search_index ADD COLUMN contained_type TEXT",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE search_index ADD COLUMN contained_local_id TEXT",
        [],
    );
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_search_contained
         ON search_index(tenant_id, contained_type, is_contained, param_name)",
        [],
    )
    .map_err(|e| migration_err(format!("create contained index: {e}")))?;
    Ok(())
}

/// Migrate from schema version 11 to version 12.
///
/// Adds the async Bulk Data Submit worker schema for databases that reached v11
/// through main before this feature branch was merged.
fn migrate_v11_to_v12(conn: &Connection) -> StorageResult<()> {
    add_bulk_submit_worker_schema(conn)
}

/// Migrate from schema version 12 to version 13.
///
/// Adds the `user_settings` table that backs the per-user UI settings store
/// (theme, default tenant, active FHIR version, recent queries, …). One opaque
/// JSON document is stored per user, keyed by `user_key`, with a monotonic
/// `version` for optimistic locking. This table is intentionally independent of
/// the FHIR `resources` table so UI preferences never leak into FHIR machinery.
fn migrate_v12_to_v13(conn: &Connection) -> StorageResult<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS user_settings (
            user_key   TEXT NOT NULL PRIMARY KEY,
            data       BLOB NOT NULL,
            version    INTEGER NOT NULL DEFAULT 1,
            updated_at TEXT NOT NULL
        )",
        [],
    )
    .map_err(|e| migration_err(format!("create user_settings table: {e}")))?;
    Ok(())
}

/// Migrate from schema version 13 to version 14.
///
/// Adds the tenant registry: a canonical list of first-class tenants backing
/// the admin tenant-maintenance API (list / add / delete). Until now a tenant
/// was only ever an implicit identifier string; this table records the tenants
/// that have been explicitly provisioned, with an optional human-friendly
/// display name and a creation timestamp. Tenants that merely have data but
/// were never registered are still discoverable via a `GROUP BY tenant_id` on
/// `resources`; the registry adds the metadata that data alone cannot provide.
fn migrate_v13_to_v14(conn: &Connection) -> StorageResult<()> {
    ensure_tenants_table(conn)
}

/// Migrate from schema version 14 to version 15.
///
/// Sweeps `resource_fts` rows orphaned by the pre-v15 purge paths (issue #386).
///
/// `resource_fts` is an FTS5 *virtual* table, so it can carry no foreign key and
/// the `ON DELETE CASCADE` from `resources` never reached it — and no purge path
/// deleted from it explicitly. Every database that has ever served `$purge`,
/// type-level `$purge`, or a tenant purge is therefore still holding the
/// narrative text and the complete serialized body of resources an operator was
/// told had been removed. The code fix stops new orphans; this removes the ones
/// already on disk, which is the half that matters to anyone running today.
///
/// Soft-deleted resources keep their `resources` row, so `NOT EXISTS` correctly
/// preserves their entries — matching current behaviour, where a soft delete
/// does not drop the FTS row.
///
/// The single statement runs in its own implicit transaction. It scans
/// `resource_fts` once (unavoidable: its key columns are `UNINDEXED`, and an
/// FTS5 virtual table admits no auxiliary index) and probes `resources` by
/// primary key.
fn migrate_v14_to_v15(conn: &Connection) -> StorageResult<()> {
    // FTS5 is optional at compile time; a database built without it has no
    // table to sweep.
    let fts_exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='resource_fts'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if !fts_exists {
        return Ok(());
    }

    let swept = conn
        .execute(
            "DELETE FROM resource_fts
              WHERE NOT EXISTS (
                  SELECT 1 FROM resources r
                   WHERE r.tenant_id     = resource_fts.tenant_id
                     AND r.resource_type = resource_fts.resource_type
                     AND r.id            = resource_fts.resource_id)",
            [],
        )
        .map_err(|e| migration_err(format!("v15 resource_fts orphan sweep: {e}")))?;

    if swept > 0 {
        tracing::info!(
            orphaned_fts_rows = swept,
            "Swept full-text rows left behind by pre-v15 purges (issue #386)"
        );
    }
    Ok(())
}

/// v15 -> v16 migration: `bulk_entry_results` gains `file_url` in its primary
/// key (#457). Line numbers restart in every manifest output file, so without
/// the file in the key every file after the first collided on its first entry.
/// SQLite cannot alter a primary key, so the table is rebuilt; pre-migration
/// rows keep an empty file_url.
fn migrate_v15_to_v16(conn: &Connection) -> StorageResult<()> {
    conn.execute_batch(
        "CREATE TABLE bulk_entry_results_v16 (
            tenant_id TEXT NOT NULL,
            submitter TEXT NOT NULL,
            submission_id TEXT NOT NULL,
            manifest_id TEXT NOT NULL,
            file_url TEXT NOT NULL DEFAULT '',
            line_number INTEGER NOT NULL,
            resource_type TEXT NOT NULL,
            resource_id TEXT,
            created INTEGER,
            outcome TEXT NOT NULL,
            operation_outcome BLOB,
            PRIMARY KEY (tenant_id, submitter, submission_id, manifest_id, file_url, line_number),
            FOREIGN KEY (tenant_id, submitter, submission_id, manifest_id)
                REFERENCES bulk_manifests(tenant_id, submitter, submission_id, manifest_id) ON DELETE CASCADE
        );
        INSERT INTO bulk_entry_results_v16
            (tenant_id, submitter, submission_id, manifest_id, line_number, resource_type, resource_id, created, outcome, operation_outcome)
        SELECT tenant_id, submitter, submission_id, manifest_id, line_number, resource_type, resource_id, created, outcome, operation_outcome
        FROM bulk_entry_results;
        DROP TABLE bulk_entry_results;
        ALTER TABLE bulk_entry_results_v16 RENAME TO bulk_entry_results;
        CREATE INDEX IF NOT EXISTS idx_bulk_entry_results_outcome
            ON bulk_entry_results(tenant_id, submitter, submission_id, manifest_id, outcome);",
    )
    .map_err(|e| migration_err(format!("migrate bulk_entry_results to v16: {e}")))
}

fn migration_err(message: String) -> crate::error::StorageError {
    crate::error::StorageError::Backend(crate::error::BackendError::Internal {
        backend_name: "sqlite".to_string(),
        message,
        source: None,
    })
}

/// Migrate from schema version 16 to version 17.
///
/// Adds the `bulk_provider_submissions` table backing the provider-side Bulk
/// Submit store (#772): the submissions the Bulk Import workspace sends,
/// previously misfiled in the per-user `user_settings` document. One opaque
/// JSON document per (tenant, submission), whole-document writes under a
/// monotonic `version` for optimistic locking.
fn migrate_v16_to_v17(conn: &Connection) -> StorageResult<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS bulk_provider_submissions (
            tenant_id  TEXT NOT NULL,
            id         TEXT NOT NULL,
            data       BLOB NOT NULL,
            version    INTEGER NOT NULL DEFAULT 1,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (tenant_id, id)
        )",
        [],
    )
    .map_err(|e| migration_err(format!("create bulk_provider_submissions table: {e}")))?;
    Ok(())
}

/// Migrate from schema version 17 to version 18: byte-level ingest
/// progress on manifests, so the status endpoint can report a real
/// percentage while a file streams in.
fn migrate_v17_to_v18(conn: &Connection) -> StorageResult<()> {
    // The columns already exist when the table was created fresh at v18 — guard
    // with PRAGMA table_info since SQLite has no `ADD COLUMN IF NOT EXISTS`.
    let manifest_columns: Vec<String> = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(bulk_manifests)")
            .map_err(|e| migration_err(format!("pragma bulk_manifests: {e}")))?;
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| migration_err(format!("pragma rows: {e}")))?
            .filter_map(|r| r.ok())
            .collect();
        cols
    };
    let adds = [
        (
            "bytes_processed",
            "ALTER TABLE bulk_manifests ADD COLUMN bytes_processed INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "bytes_total",
            "ALTER TABLE bulk_manifests ADD COLUMN bytes_total INTEGER NOT NULL DEFAULT 0",
        ),
    ];
    for (col, sql) in &adds {
        if !manifest_columns.iter().any(|c| c == col) {
            conn.execute(sql, [])
                .map_err(|e| migration_err(format!("add manifest byte columns: {e}")))?;
        }
    }
    Ok(())
}

/// v19: keyset-pagination index for reindex page fetches.
///
/// fetch_resources_page orders by (last_updated, id) within a
/// (tenant_id, resource_type) — with no covering index every page pays a
/// sort of the whole type, which turns a 5.7M-row Observation reindex
/// quadratic (#903).
fn migrate_v18_to_v19(conn: &Connection) -> StorageResult<()> {
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_resources_reindex          ON resources(tenant_id, resource_type, last_updated, id)",
        [],
    )
    .map_err(|e| migration_err(format!("create idx_resources_reindex: {e}")))?;
    Ok(())
}

/// Drop all tables (for testing).
#[cfg(test)]
#[allow(dead_code)]
pub fn drop_all_tables(conn: &Connection) -> StorageResult<()> {
    // Drop FTS5 table first (if exists)
    let _ = conn.execute("DROP TABLE IF EXISTS resource_fts", []);
    let _ = conn.execute("DROP TABLE IF EXISTS search_index_fts", []);

    // Drop bulk tables (order matters due to foreign keys)
    let _ = conn.execute("DROP TABLE IF EXISTS bulk_submission_changes", []);
    let _ = conn.execute("DROP TABLE IF EXISTS bulk_entry_results", []);
    let _ = conn.execute("DROP TABLE IF EXISTS bulk_manifests", []);
    let _ = conn.execute("DROP TABLE IF EXISTS bulk_submissions", []);
    let _ = conn.execute("DROP TABLE IF EXISTS bulk_export_files", []);
    let _ = conn.execute("DROP TABLE IF EXISTS bulk_export_progress", []);
    let _ = conn.execute("DROP TABLE IF EXISTS bulk_export_jobs", []);

    conn.execute("DROP TABLE IF EXISTS search_index", [])
        .map_err(|e| {
            crate::error::StorageError::Backend(crate::error::BackendError::Internal {
                backend_name: "sqlite".to_string(),
                message: format!("Failed to drop search_index: {}", e),
                source: None,
            })
        })?;
    conn.execute("DROP TABLE IF EXISTS resource_history", [])
        .map_err(|e| {
            crate::error::StorageError::Backend(crate::error::BackendError::Internal {
                backend_name: "sqlite".to_string(),
                message: format!("Failed to drop resource_history: {}", e),
                source: None,
            })
        })?;
    conn.execute("DROP TABLE IF EXISTS resources", [])
        .map_err(|e| {
            crate::error::StorageError::Backend(crate::error::BackendError::Internal {
                backend_name: "sqlite".to_string(),
                message: format!("Failed to drop resources: {}", e),
                source: None,
            })
        })?;
    conn.execute("DROP TABLE IF EXISTS schema_version", [])
        .map_err(|e| {
            crate::error::StorageError::Backend(crate::error::BackendError::Internal {
                backend_name: "sqlite".to_string(),
                message: format!("Failed to drop schema_version: {}", e),
                source: None,
            })
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_initialization() {
        let conn = Connection::open_in_memory().unwrap();
        initialize_schema(&conn).unwrap();

        // Verify tables exist
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(tables.contains(&"resources".to_string()));
        assert!(tables.contains(&"resource_history".to_string()));
        assert!(tables.contains(&"search_index".to_string()));
        assert!(tables.contains(&"schema_version".to_string()));
        // The tenant registry (schema v14) is created on a fresh init.
        assert!(tables.contains(&"tenants".to_string()));
    }

    #[test]
    fn test_migration_creates_tenants_table_on_upgrade() {
        // Build a pre-registry schema (no `tenants` table) then upgrade it exactly
        // as `initialize_schema` would for an existing database.
        let conn = Connection::open_in_memory().unwrap();
        create_schema_v1(&conn).unwrap();
        let _ = get_schema_version(&conn).unwrap();
        migrate_v1_to_v2(&conn).unwrap();
        migrate_v2_to_v3(&conn).unwrap();
        migrate_v3_to_v4(&conn).unwrap();
        migrate_v4_to_v5(&conn).unwrap();
        migrate_v5_to_v6(&conn).unwrap();
        migrate_v6_to_v7(&conn).unwrap();
        migrate_v7_to_v8(&conn).unwrap();
        migrate_v8_to_v9(&conn).unwrap();
        migrate_v9_to_v10(&conn).unwrap();
        migrate_v10_to_v11(&conn).unwrap();
        migrate_v11_to_v12(&conn).unwrap();
        migrate_v12_to_v13(&conn).unwrap();
        set_schema_version(&conn, 13).unwrap();

        // No tenants table before the upgrade.
        let before: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='tenants'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(before, 0);

        // The same entry point the server uses on an existing database.
        initialize_schema(&conn).unwrap();

        assert_eq!(get_schema_version(&conn).unwrap(), SCHEMA_VERSION);
        let after: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='tenants'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(after, 1, "the upgrade must create the tenants table");
    }

    #[test]
    fn test_initialize_schema_self_heals_missing_tenants_table() {
        // Simulate a database left at the registry's version by a pre-release
        // build that stamped the version but never created the table (the
        // version-gated migration then can't re-run). initialize_schema restores it.
        let conn = Connection::open_in_memory().unwrap();
        initialize_schema(&conn).unwrap();
        conn.execute("DROP TABLE tenants", []).unwrap();
        assert_eq!(get_schema_version(&conn).unwrap(), SCHEMA_VERSION);

        // Re-init (as a server restart would) heals it without a version change.
        initialize_schema(&conn).unwrap();

        let count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='tenants'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 1,
            "initialize_schema must self-heal the tenants table"
        );
    }

    #[test]
    fn test_schema_version() {
        let conn = Connection::open_in_memory().unwrap();
        initialize_schema(&conn).unwrap();

        let version = get_schema_version(&conn).unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn test_schema_idempotent() {
        let conn = Connection::open_in_memory().unwrap();

        // Initialize twice - should not fail
        initialize_schema(&conn).unwrap();
        initialize_schema(&conn).unwrap();

        let version = get_schema_version(&conn).unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    /// Migrations must be re-runnable: a database already carrying the latest
    /// columns can be replayed from any earlier recorded version (tests do this,
    /// and so does a build that stamped a version before finishing its work).
    /// SQLite has no `ADD COLUMN IF NOT EXISTS`, so every `ALTER TABLE ... ADD
    /// COLUMN` step has to guard against the column already being there.
    #[test]
    fn test_migration_ladder_replays_on_a_current_database() {
        for from in 1..SCHEMA_VERSION {
            let conn = Connection::open_in_memory().unwrap();
            initialize_schema(&conn).unwrap();
            set_schema_version(&conn, from).unwrap();
            initialize_schema(&conn)
                .unwrap_or_else(|e| panic!("replay from v{from} failed: {e:?}"));
            assert_eq!(get_schema_version(&conn).unwrap(), SCHEMA_VERSION);
        }
    }

    #[test]
    fn test_bulk_tables_exist() {
        let conn = Connection::open_in_memory().unwrap();
        initialize_schema(&conn).unwrap();

        // Verify bulk export tables exist
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        // Bulk export tables
        assert!(tables.contains(&"bulk_export_jobs".to_string()));
        assert!(tables.contains(&"bulk_export_progress".to_string()));
        assert!(tables.contains(&"bulk_export_files".to_string()));

        // Bulk submit tables
        assert!(tables.contains(&"bulk_submissions".to_string()));
        assert!(tables.contains(&"bulk_manifests".to_string()));
        assert!(tables.contains(&"bulk_entry_results".to_string()));
        assert!(tables.contains(&"bulk_submission_changes".to_string()));
    }

    #[test]
    fn test_migration_v5_to_v6() {
        let conn = Connection::open_in_memory().unwrap();

        // Create schema at version 5 (without bulk tables)
        create_schema_v1(&conn).unwrap();
        // Initialize schema_version table via get_schema_version
        let _ = get_schema_version(&conn).unwrap();
        migrate_v1_to_v2(&conn).unwrap();
        migrate_v2_to_v3(&conn).unwrap();
        migrate_v3_to_v4(&conn).unwrap();
        migrate_v4_to_v5(&conn).unwrap();
        set_schema_version(&conn, 5).unwrap();

        // Verify bulk tables don't exist yet
        let table_count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name LIKE 'bulk_%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 0);

        // Run migration
        migrate_v5_to_v6(&conn).unwrap();

        // Verify bulk tables now exist
        let table_count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name LIKE 'bulk_%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 7); // 3 export + 4 submit tables
    }

    #[test]
    fn test_migration_v8_to_v9_adds_submit_worker_columns() {
        let conn = Connection::open_in_memory().unwrap();
        create_schema_v1(&conn).unwrap();
        let _ = get_schema_version(&conn).unwrap();
        migrate_v1_to_v2(&conn).unwrap();
        migrate_v2_to_v3(&conn).unwrap();
        migrate_v3_to_v4(&conn).unwrap();
        migrate_v4_to_v5(&conn).unwrap();
        migrate_v5_to_v6(&conn).unwrap();
        migrate_v6_to_v7(&conn).unwrap();
        migrate_v7_to_v8(&conn).unwrap();
        set_schema_version(&conn, 8).unwrap();

        migrate_v8_to_v9(&conn).unwrap();

        let has_column = |table: &str, col: &str| -> bool {
            let mut stmt = conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap();
            stmt.query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .filter_map(|r| r.ok())
                .any(|c| c == col)
        };

        for col in [
            "owner_subject",
            "poll_token",
            "transaction_time",
            "requires_access_token",
            "request_url",
        ] {
            assert!(has_column("bulk_submissions", col), "missing {col}");
        }
        for col in [
            "worker_id",
            "lease_expiry",
            "fencing_token",
            "fhir_base_url",
            "output_format",
            "file_request_headers",
            "oauth_metadata_urls",
            "file_encryption_key",
            "last_processed_line",
            "import_directives",
            "submission_metadata",
        ] {
            assert!(has_column("bulk_manifests", col), "missing {col}");
        }

        let files_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='bulk_submit_files'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .unwrap()
            > 0;
        assert!(files_exists);

        // Idempotent re-run (mirrors initialize_schema running it on a fresh DB).
        migrate_v8_to_v9(&conn).unwrap();
    }

    #[test]
    fn test_migration_v7_to_v8_backfills_duplicate_file_rows() {
        // Build a v6/v7-era schema (bulk tables without the v8 lease/part columns).
        let conn = Connection::open_in_memory().unwrap();
        create_schema_v1(&conn).unwrap();
        let _ = get_schema_version(&conn).unwrap();
        migrate_v1_to_v2(&conn).unwrap();
        migrate_v2_to_v3(&conn).unwrap();
        migrate_v3_to_v4(&conn).unwrap();
        migrate_v4_to_v5(&conn).unwrap();
        migrate_v5_to_v6(&conn).unwrap();
        migrate_v6_to_v7(&conn).unwrap();
        set_schema_version(&conn, 7).unwrap();

        // Seed a job and THREE output files for the same (job, file_type,
        // resource_type) — all default part_index would collide.
        conn.execute(
            "INSERT INTO bulk_export_jobs
             (id, tenant_id, status, level, request_json, transaction_time, created_at)
             VALUES ('j1', 't1', 'complete', 'system', '{}', '2026-01-01T00:00:00Z',
                     '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        for i in 0..3 {
            conn.execute(
                "INSERT INTO bulk_export_files
                 (job_id, resource_type, file_type, file_path, resource_count, byte_count)
                 VALUES ('j1', 'Patient', 'output', ?1, 10, 100)",
                rusqlite::params![format!("/exports/j1/Patient-{i}.ndjson")],
            )
            .unwrap();
        }

        // Run the v7 -> v8 migration.
        migrate_v7_to_v8(&conn).unwrap();

        // The backfill must have produced distinct 0-based part_index values
        // per group, so the unique index built without a collision.
        let mut stmt = conn
            .prepare(
                "SELECT part_index FROM bulk_export_files
                 WHERE job_id = 'j1' ORDER BY part_index",
            )
            .unwrap();
        let part_indexes: Vec<i64> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(part_indexes, vec![0, 1, 2]);

        // The unique index exists.
        let idx_count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='index' AND name='idx_export_files_part'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(idx_count, 1);

        // Re-running the migration is a no-op (idempotent).
        migrate_v7_to_v8(&conn).unwrap();

        // New lease columns are present on bulk_export_jobs.
        let has_worker_id: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('bulk_export_jobs')
                 WHERE name='worker_id'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(has_worker_id, 1);
    }
}
