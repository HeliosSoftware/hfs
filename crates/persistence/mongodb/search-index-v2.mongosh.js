// Generated from crates/persistence/src/backends/mongodb/search_index_catalog.rs.
// Do not edit by hand: a unit test compares this file to the catalog.
// Usage: mongosh "$HFS_MONGODB_URL" --file <this file>, with HFS_MONGODB_DATABASE exported
// Builds every index in one collection scan; reads and writes continue meanwhile.
if (!process.env.HFS_MONGODB_DATABASE) {
  throw new Error("HFS_MONGODB_DATABASE is not set: export the name of the database HFS uses (default helios)");
}
db = db.getSiblingDB(process.env.HFS_MONGODB_DATABASE);
db.runCommand({
  "createIndexes": "search_index",
  "indexes": [
    {
      "key": {
        "tenant_id": 1,
        "resource_type": 1,
        "param_name": 1,
        "value_string": 1,
        "resource_id": 1
      },
      "name": "idx_search_string_v2",
      "partialFilterExpression": {
        "value_string": {
          "$exists": true
        }
      }
    },
    {
      "key": {
        "tenant_id": 1,
        "resource_type": 1,
        "param_name": 1,
        "value_token_code": 1,
        "value_token_system": 1,
        "resource_id": 1
      },
      "name": "idx_search_token_v2",
      "partialFilterExpression": {
        "value_token_code": {
          "$exists": true
        }
      }
    },
    {
      "key": {
        "tenant_id": 1,
        "resource_type": 1,
        "param_name": 1,
        "value_date": 1,
        "value_date_end": 1,
        "resource_id": 1
      },
      "name": "idx_search_date_v3",
      "partialFilterExpression": {
        "value_date": {
          "$exists": true
        }
      }
    },
    {
      "key": {
        "tenant_id": 1,
        "resource_type": 1,
        "param_name": 1,
        "value_number": 1,
        "resource_id": 1
      },
      "name": "idx_search_number_v2",
      "partialFilterExpression": {
        "value_number": {
          "$exists": true
        }
      }
    },
    {
      "key": {
        "tenant_id": 1,
        "resource_type": 1,
        "param_name": 1,
        "value_quantity_value": 1,
        "value_quantity_unit": 1,
        "resource_id": 1
      },
      "name": "idx_search_quantity_v2",
      "partialFilterExpression": {
        "value_quantity_value": {
          "$exists": true
        }
      }
    },
    {
      "key": {
        "tenant_id": 1,
        "resource_type": 1,
        "param_name": 1,
        "value_reference": 1,
        "resource_id": 1
      },
      "name": "idx_search_reference_v2",
      "partialFilterExpression": {
        "value_reference": {
          "$exists": true
        }
      }
    },
    {
      "key": {
        "tenant_id": 1,
        "resource_type": 1,
        "param_name": 1,
        "value_uri": 1,
        "resource_id": 1
      },
      "name": "idx_search_uri_v2",
      "partialFilterExpression": {
        "value_uri": {
          "$exists": true
        }
      }
    },
    {
      "key": {
        "tenant_id": 1,
        "resource_type": 1,
        "param_name": 1,
        "value_token_display": 1,
        "resource_id": 1
      },
      "name": "idx_search_token_display_v2",
      "partialFilterExpression": {
        "value_token_display": {
          "$exists": true
        }
      }
    },
    {
      "key": {
        "tenant_id": 1,
        "resource_type": 1,
        "param_name": 1,
        "value_identifier_type_system": 1,
        "value_identifier_type_code": 1,
        "resource_id": 1
      },
      "name": "idx_search_identifier_type_v2",
      "partialFilterExpression": {
        "value_identifier_type_system": {
          "$exists": true
        }
      }
    }
  ]
});
