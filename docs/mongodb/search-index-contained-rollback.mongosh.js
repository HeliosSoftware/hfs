// Generated from crates/persistence/src/backends/mongodb/search_index_catalog.rs.
// Do not edit by hand: a unit test compares this file to the catalog.
// Usage: mongosh "$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE" <this file>
// Copies contained rows back into search_index (with is_contained: true) and
// recreates the generation-2 partial index, for a rollback to a binary that
// predates search_index_contained. Idempotent: rows already present are skipped.
db.search_index_contained.find().forEach(function (row) {
  row.is_contained = true;
  try { db.search_index.insertOne(row); } catch (e) { if (e.code !== 11000) throw e; }
});
// Reset the migration record so a later generation-3 boot re-runs the move: its
// inserts are duplicate-key no-ops for rows already copied back above, and it then
// deletes the source rows this script just restored.
db.schema_version.updateOne(
  { _id: "schema_version" },
  { $unset: { "search_indexes.contained_rows_moved": "" }, $set: { "search_indexes.generation": 2 } }
);
db.runCommand({
  "createIndexes": "search_index",
  "indexes": [
    {
      "key": {
        "tenant_id": 1,
        "contained_type": 1,
        "is_contained": 1,
        "param_name": 1,
        "resource_type": 1,
        "resource_id": 1,
        "contained_local_id": 1
      },
      "name": "idx_search_contained",
      "partialFilterExpression": {
        "is_contained": true
      }
    }
  ]
});
