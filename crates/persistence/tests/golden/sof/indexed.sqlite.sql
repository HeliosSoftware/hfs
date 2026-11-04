SELECT
  CAST((SELECT json_extract(fe.value, '$.family') FROM json_each(CASE WHEN json_type(CAST(r.data AS TEXT), '$.name') = 'array' THEN json_extract(CAST(r.data AS TEXT), '$.name') WHEN json_type(CAST(r.data AS TEXT), '$.name') IN ('object', 'array') THEN json_array(json(json_extract(CAST(r.data AS TEXT), '$.name'))) WHEN json_type(CAST(r.data AS TEXT), '$.name') IS NOT NULL THEN json_array(json_extract(CAST(r.data AS TEXT), '$.name')) ELSE '[]' END) fe LIMIT 1 OFFSET 1) AS TEXT) AS "family"
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
    "family"
  ],
  "decodes": [
    "Text"
  ],
  "client_limit": null
}
*/
