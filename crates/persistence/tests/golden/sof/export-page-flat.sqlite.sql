SELECT
  json_extract(CAST(r.data AS TEXT), '$.id') AS "id",
  CASE WHEN (json_extract(CAST(r.data AS TEXT), '$.active')) THEN 'true' WHEN NOT (json_extract(CAST(r.data AS TEXT), '$.active')) THEN 'false' END AS "active"
FROM resources r
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
  AND r.last_updated >= ?3
  AND r.id COLLATE BINARY > ?4
  AND r.id COLLATE BINARY <= ?5
ORDER BY r.id COLLATE BINARY

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
      "type": "text",
      "value": ""
    },
    {
      "type": "text",
      "value": ""
    }
  ],
  "columns": [
    "id",
    "active"
  ],
  "decodes": [
    "Text",
    "Boolean"
  ],
  "client_limit": 50
}
*/
