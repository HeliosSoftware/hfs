SELECT
  json_extract(r.data, '$.id') AS "observation_id",
  coalesce(json_extract(fe.value, '$.code[0].text'), json_extract(fe.value, '$.code.text')) AS "component_name",
  json_extract(fe.value, '$.code.coding[0].code') AS "component_code"
FROM resources r
JOIN json_each(CASE WHEN json_type(r.data, '$.component') = 'array' THEN json_extract(r.data, '$.component') WHEN json_type(r.data, '$.component') IN ('object', 'array') THEN json_array(json(json_extract(r.data, '$.component'))) WHEN json_type(r.data, '$.component') IS NOT NULL THEN json_array(json_extract(r.data, '$.component')) ELSE '[]' END) fe ON 1=1
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
      "value": "Observation"
    }
  ],
  "columns": [
    "observation_id",
    "component_name",
    "component_code"
  ],
  "decodes": [
    "Text",
    "Text",
    "Text"
  ],
  "client_limit": null
}
*/
