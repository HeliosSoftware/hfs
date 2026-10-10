SELECT
  CAST((SELECT json_extract(w0.value, '$.family') FROM json_each(r.data, '$.name') w0 WHERE (json_extract(w0.value, '$.use') = 'official') ORDER BY COALESCE(w0.rowid, -1) LIMIT 1) AS TEXT) AS "pick"
FROM resources r
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
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
    }
  ],
  "columns": [
    "pick"
  ],
  "decodes": [
    "Text"
  ],
  "client_limit": null
}
*/
