SELECT
  json_extract(CAST(r.data AS TEXT), '$.id') AS "r",
  json_extract(fe.value, '$.family') AS "u0"
FROM resources r
JOIN json_each(CAST(r.data AS TEXT), '$.name') fe ON 1=1
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
ORDER BY r.last_updated, r.id COLLATE BINARY, COALESCE(fe.rowid, -1)

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
    "r",
    "u0"
  ],
  "decodes": [
    "Text",
    "Text"
  ],
  "client_limit": null
}
*/
