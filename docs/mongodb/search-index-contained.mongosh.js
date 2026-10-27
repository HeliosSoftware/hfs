// Generated from crates/persistence/src/backends/mongodb/search_index_catalog.rs.
// Do not edit by hand: a unit test compares this file to the catalog.
// Usage: mongosh "$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE" <this file>
// Builds every index in one collection scan; reads and writes continue meanwhile.
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
