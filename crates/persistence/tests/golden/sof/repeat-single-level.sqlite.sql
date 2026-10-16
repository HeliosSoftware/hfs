SELECT
  CAST(r.id AS TEXT) AS "observation_id",
  coalesce(json_extract(fe.value, '$.code[0].text'), json_extract(fe.value, '$.code.text')) AS "component_name",
  json_extract(fe.value, '$.code.coding[0].code') AS "component_code"
FROM resources r
JOIN json_each(r.data, '$.component') fe ON 1=1
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
