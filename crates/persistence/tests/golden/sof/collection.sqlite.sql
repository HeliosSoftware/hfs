SELECT
  CAST((SELECT json_group_array(ca0.value ORDER BY COALESCE(ca0.rowid, -1)) FROM json_each(r.data, '$.name') ca0) AS TEXT) AS "objects",
  CAST((SELECT json_group_array(ca1.value ORDER BY COALESCE(ca0.rowid, -1), COALESCE(ca1.rowid, -1)) FROM json_each(r.data, '$.name') ca0, json_each(CASE WHEN json_type(ca0.value, '$.family') = 'array' THEN json_extract(ca0.value, '$.family') ELSE json_array(json_extract(ca0.value, '$.family')) END) ca1 WHERE json_type(ca0.value, '$.family') IS NOT NULL) AS TEXT) AS "families",
  CAST((SELECT json_group_array(ca1.value ORDER BY COALESCE(ca0.rowid, -1), COALESCE(ca1.rowid, -1)) FROM json_each(r.data, '$.name') ca0, json_each(CASE WHEN json_type(ca0.value, '$.given') = 'array' THEN json_extract(ca0.value, '$.given') ELSE json_array(json_extract(ca0.value, '$.given')) END) ca1 WHERE json_type(ca0.value, '$.given') IS NOT NULL) AS TEXT) AS "givens"
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
    "objects",
    "families",
    "givens"
  ],
  "decodes": [
    "Json",
    "Json",
    "Json"
  ],
  "client_limit": null
}
*/
