SELECT
  json_extract(fe.value, '$.family') AS "family",
  CAST(COALESCE(CAST(fe.key AS INTEGER), 0) AS INTEGER) AS "index"
FROM resources r
JOIN json_each(r.data, '$.name') fe ON 1=1
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
    "family",
    "index"
  ],
  "decodes": [
    "Text",
    "Integer"
  ],
  "client_limit": null
}
*/
