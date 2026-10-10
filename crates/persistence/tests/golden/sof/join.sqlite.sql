SELECT
  CAST((SELECT group_concat(ja1.value, '|' ORDER BY COALESCE(ja0.rowid, -1), COALESCE(ja1.rowid, -1)) FROM json_each(r.data, '$.name') ja0, json_each(CASE WHEN json_type(ja0.value, '$.given') = 'array' THEN json_extract(ja0.value, '$.given') WHEN json_type(ja0.value, '$.given') IN ('object', 'array') THEN json_array(json(json_extract(ja0.value, '$.given'))) WHEN json_type(ja0.value, '$.given') IS NOT NULL THEN json_array(json_extract(ja0.value, '$.given')) ELSE '[]' END) ja1) AS TEXT) AS "joined"
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
    "joined"
  ],
  "decodes": [
    "Text"
  ],
  "client_limit": null
}
*/
