SELECT
  json_extract(r.data, '$.id') AS "id",
  CASE WHEN (json_extract(r.data, '$.active')) THEN 'true' WHEN NOT (json_extract(r.data, '$.active')) THEN 'false' END AS "active"
FROM resources r
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
  AND r.last_updated >= ?3
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
      "value": "2024-01-01T12:34:56.123456789+00:00"
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
  "client_limit": null
}
*/
