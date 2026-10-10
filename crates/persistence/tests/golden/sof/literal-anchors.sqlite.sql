SELECT
  'resources r' AS "resource_literal",
  'r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0' AS "sqlite_anchor",
  'r.tenant_id = $1
  AND r.resource_type = $2
  AND r.is_deleted = false' AS "pg_anchor"
FROM resources r
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
  AND r.last_updated >= ?3
  AND r.id IN (SELECT value FROM json_each(?4))
ORDER BY r.last_updated, r.id COLLATE BINARY

/* Runner bindings and output metadata:
{
  "bindings": [
    {
      "type": "text",
      "value": "golden-tenant"
    },
    {
      "type": "text",
      "value": "Patient"
    },
    {
      "type": "timestamp",
      "value": "2024-01-01T00:00:00+00:00"
    },
    {
      "type": "text-list",
      "value": [
        "p1"
      ]
    }
  ],
  "columns": [
    "resource_literal",
    "sqlite_anchor",
    "pg_anchor"
  ],
  "decodes": [
    "Text",
    "Text",
    "Text"
  ],
  "client_limit": null
}
*/
