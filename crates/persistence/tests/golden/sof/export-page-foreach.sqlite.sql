SELECT
  json_extract(r.data, '$.id') AS "r",
  json_extract(fe.value, '$.family') AS "u0"
FROM resources r
JOIN json_each(r.data, '$.name') fe ON 1=1
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
  AND r.last_updated >= ?3
  AND r.id COLLATE BINARY > ?4
  AND r.id COLLATE BINARY <= ?5
ORDER BY r.id COLLATE BINARY, COALESCE(fe.rowid, -1)

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
    "r",
    "u0"
  ],
  "decodes": [
    "Text",
    "Text"
  ],
  "client_limit": 50
}
*/
