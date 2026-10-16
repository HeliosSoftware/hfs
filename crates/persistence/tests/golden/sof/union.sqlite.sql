SELECT
  json_extract(r.data, '$.id') AS "value"
FROM resources r
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
UNION ALL
SELECT
  json_extract(fe.value, '$.family') AS "value"
FROM resources r
JOIN json_each(CASE WHEN json_type(r.data, '$.name') = 'array' THEN json_extract(r.data, '$.name') WHEN json_type(r.data, '$.name') IN ('object', 'array') THEN json_array(json(json_extract(r.data, '$.name'))) WHEN json_type(r.data, '$.name') IS NOT NULL THEN json_array(json_extract(r.data, '$.name')) ELSE '[]' END) fe ON 1=1
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0

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
    "value"
  ],
  "decodes": [
    "Text"
  ],
  "client_limit": null
}
*/
