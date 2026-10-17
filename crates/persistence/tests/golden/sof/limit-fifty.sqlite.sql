SELECT
  json_extract(r.data, '$.id') AS "id",
  CASE WHEN (json_extract(r.data, '$.active')) THEN 'true' WHEN NOT (json_extract(r.data, '$.active')) THEN 'false' END AS "active"
FROM resources r
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
ORDER BY r.last_updated, r.id COLLATE BINARY
LIMIT 50

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
