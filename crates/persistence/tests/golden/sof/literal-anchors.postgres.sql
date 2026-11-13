SELECT
  'resources r' AS "resource_literal",
  'r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0' AS "sqlite_anchor",
  'r.tenant_id = $1
  AND r.resource_type = $2
  AND r.is_deleted = false' AS "pg_anchor"
FROM resources res
CROSS JOIN LATERAL (SELECT res.data #> '{}' AS data, res.id AS id OFFSET 0) r
WHERE res.tenant_id = $1
  AND res.resource_type = $2
  AND res.is_deleted = false
  AND res.last_updated >= $3
  AND res.id = ANY($4::text[])
ORDER BY res.last_updated, res.id COLLATE "C"

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
