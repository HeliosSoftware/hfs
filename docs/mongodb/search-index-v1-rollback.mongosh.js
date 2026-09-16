// Generated from crates/persistence/src/backends/mongodb/search_index_catalog.rs.
// Do not edit by hand: a unit test compares this file to the catalog.
// Usage: mongosh "$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE" <this file>
// Builds every index in one collection scan; reads and writes continue meanwhile.
db.runCommand({
  "createIndexes": "search_index",
  "indexes": [
    {
      "key": {
        "tenant_id": 1,
        "resource_type": 1,
        "param_name": 1,
        "value_string": 1
      },
      "name": "idx_search_string"
    },
    {
      "key": {
        "tenant_id": 1,
        "resource_type": 1,
        "param_name": 1,
        "value_token_system": 1,
        "value_token_code": 1
      },
      "name": "idx_search_token"
    },
    {
      "key": {
        "tenant_id": 1,
        "resource_type": 1,
        "param_name": 1,
        "value_date": 1
      },
      "name": "idx_search_date"
    },
    {
      "key": {
        "tenant_id": 1,
        "resource_type": 1,
        "param_name": 1,
        "value_number": 1
      },
      "name": "idx_search_number"
    },
    {
      "key": {
        "tenant_id": 1,
        "resource_type": 1,
        "param_name": 1,
        "value_quantity_value": 1,
        "value_quantity_unit": 1
      },
      "name": "idx_search_quantity"
    },
    {
      "key": {
        "tenant_id": 1,
        "resource_type": 1,
        "param_name": 1,
        "value_reference": 1
      },
      "name": "idx_search_reference"
    },
    {
      "key": {
        "tenant_id": 1,
        "resource_type": 1,
        "param_name": 1,
        "value_uri": 1
      },
      "name": "idx_search_uri"
    },
    {
      "key": {
        "tenant_id": 1,
        "resource_type": 1,
        "param_name": 1,
        "value_token_display": 1
      },
      "name": "idx_search_token_display"
    },
    {
      "key": {
        "tenant_id": 1,
        "resource_type": 1,
        "param_name": 1,
        "value_identifier_type_system": 1,
        "value_identifier_type_code": 1
      },
      "name": "idx_search_identifier_type"
    }
  ]
});
