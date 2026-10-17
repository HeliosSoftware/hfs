SELECT
  json_extract(r.data, '$.id') AS "id"
FROM resources r
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
  AND 1=0
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
      "value": "Library"
    }
  ],
  "columns": [
    "id"
  ],
  "decodes": [
    "Text"
  ],
  "client_limit": null
}
*/
