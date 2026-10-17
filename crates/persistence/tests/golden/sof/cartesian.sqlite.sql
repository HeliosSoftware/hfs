SELECT
  json_extract(fe.value, '$.family') AS "family",
  json_extract(fe2.value, '$.city') AS "city"
FROM resources r
JOIN json_each(r.data, '$.name') fe ON 1=1
JOIN json_each(r.data, '$.address') fe2 ON 1=1
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
ORDER BY r.last_updated, r.id COLLATE BINARY, COALESCE(fe.rowid, -1), COALESCE(fe2.rowid, -1)

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
    "city"
  ],
  "decodes": [
    "Text",
    "Text"
  ],
  "client_limit": null
}
*/
