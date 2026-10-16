SELECT
  json_extract(r.data, '$.id') AS "id",
  json_extract(fe2.value, '$.value') AS "value"
FROM resources r
JOIN json_each(CASE WHEN json_type(r.data, '$.contact') = 'array' THEN json_extract(r.data, '$.contact') WHEN json_type(r.data, '$.contact') IN ('object', 'array') THEN json_array(json(json_extract(r.data, '$.contact'))) WHEN json_type(r.data, '$.contact') IS NOT NULL THEN json_array(json_extract(r.data, '$.contact')) ELSE '[]' END) fe ON 1=1
JOIN json_each(CASE WHEN json_type(fe.value, '$.telecom') = 'array' THEN json_extract(fe.value, '$.telecom') WHEN json_type(fe.value, '$.telecom') IN ('object', 'array') THEN json_array(json(json_extract(fe.value, '$.telecom'))) WHEN json_type(fe.value, '$.telecom') IS NOT NULL THEN json_array(json_extract(fe.value, '$.telecom')) ELSE '[]' END) fe2 ON 1=1
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
UNION ALL
SELECT
  json_extract(r.data, '$.id') AS "id",
  coalesce(json_extract(fe3.value, '$.name[0].family'), json_extract(fe3.value, '$.name.family')) AS "value"
FROM resources r
JOIN json_each(CASE WHEN json_type(r.data, '$.contact') = 'array' THEN json_extract(r.data, '$.contact') WHEN json_type(r.data, '$.contact') IN ('object', 'array') THEN json_array(json(json_extract(r.data, '$.contact'))) WHEN json_type(r.data, '$.contact') IS NOT NULL THEN json_array(json_extract(r.data, '$.contact')) ELSE '[]' END) fe3 ON 1=1
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
    "id",
    "value"
  ],
  "decodes": [
    "Text",
    "Text"
  ],
  "client_limit": null
}
*/
