// Generated from crates/persistence/src/backends/mongodb/search_index_catalog.rs.
// Do not edit by hand: a unit test compares this file to the catalog.
// Usage: mongosh "$HFS_MONGODB_URL" --file <this file>, with HFS_MONGODB_DATABASE exported
// Builds every index in one collection scan; reads and writes continue meanwhile.
if (!process.env.HFS_MONGODB_DATABASE) {
  throw new Error("HFS_MONGODB_DATABASE is not set: export the name of the database HFS uses (default helios)");
}
db = db.getSiblingDB(process.env.HFS_MONGODB_DATABASE);
db.runCommand({
  "createIndexes": "search_index_contained",
  "indexes": [
    {
      "key": {
        "tenant_id": 1,
        "contained_type": 1,
        "param_name": 1,
        "resource_type": 1,
        "resource_id": 1,
        "contained_local_id": 1
      },
      "name": "idx_search_contained"
    },
    {
      "key": {
        "tenant_id": 1,
        "resource_type": 1,
        "resource_id": 1
      },
      "name": "idx_search_contained_resource"
    },
    {
      "key": {
        "tenant_id": 1,
        "contained_type": 1,
        "param_name": 1,
        "composite_slot": 1
      },
      "name": "idx_search_contained_composite_slot_probe",
      "partialFilterExpression": {
        "composite_group": {
          "$exists": true
        }
      }
    }
  ]
});
