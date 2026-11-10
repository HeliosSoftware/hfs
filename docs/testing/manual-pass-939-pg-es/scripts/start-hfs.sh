#!/bin/bash
# Real pg-es server for the #939 pass. Runs from the worktree root so ./data (search parameters) loads.
W=$HOME/tmp/939/work
cd /home/edson/src/hfs-wt/docs-939-manual-pass-screenshots
export HFS_SERVER_HOST=127.0.0.1 HFS_SERVER_PORT=18080 HFS_BASE_URL=http://localhost:18080
export HFS_LOG_LEVEL=${HFS_LOG_LEVEL:-info} HFS_DEFAULT_FHIR_VERSION=R4 HFS_REQUEST_TIMEOUT=600 HFS_SUBSCRIPTIONS_ENABLED=${HFS_SUBSCRIPTIONS_ENABLED:-true}
export HFS_BULK_EXPORT_OUTPUT_DIR=$W/bulk-exports HFS_EXPORT_DIR=$W/sql-exports
export HFS_COMPOSITE_SYNC_MODE=synchronous HFS_ELASTICSEARCH_WRITE_REFRESH=wait_for HFS_ELASTICSEARCH_REINDEX_REFRESH=false
export HFS_STORAGE_BACKEND=pg-es HFS_DATABASE_URL=postgresql://helios:helios@localhost:18932/helios HFS_ELASTICSEARCH_NODES=http://localhost:18921
export HFS_ELASTICSEARCH_INDEX_PREFIX=$(cat $W/es_prefix)
# T3 settings of the full-corpus run (comment #939, 2026-09-27)
export HFS_BULK_SUBMIT_DEFER_INDEXING=false HFS_BULK_SUBMIT_INDEX_QUEUE=64 HFS_BULK_SUBMIT_INDEX_CONCURRENCY=8 HFS_BULK_SUBMIT_INDEX_COALESCE=4
export HFS_BULK_SUBMIT_FILE_CONCURRENCY=4 HFS_BULK_SUBMIT_BATCH_SIZE=1000 HFS_PG_MAX_CONNECTIONS=32
exec ./target/release/hfs
