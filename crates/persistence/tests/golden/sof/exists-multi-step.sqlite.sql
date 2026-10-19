SELECT
  json_extract(r.data, '$.id') AS "id"
FROM resources r
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
  AND (EXISTS (SELECT 1 FROM json_each(CASE WHEN json_type(r.data, '$.contact') = 'array' THEN json_extract(r.data, '$.contact') WHEN json_type(r.data, '$.contact') IN ('object', 'array') THEN json_array(json(json_extract(r.data, '$.contact'))) WHEN json_type(r.data, '$.contact') IS NOT NULL THEN json_array(json_extract(r.data, '$.contact')) ELSE '[]' END) w0_0, json_each(CASE WHEN json_type(w0_0.value, '$.telecom') = 'array' THEN json_extract(w0_0.value, '$.telecom') WHEN json_type(w0_0.value, '$.telecom') IN ('object', 'array') THEN json_array(json(json_extract(w0_0.value, '$.telecom'))) WHEN json_type(w0_0.value, '$.telecom') IS NOT NULL THEN json_array(json_extract(w0_0.value, '$.telecom')) ELSE '[]' END) w0 WHERE ((json_extract(w0.value, '$.system') = 'phone') AND (json_extract(w0.value, '$.value') = '555')))) NOT IN (0, 'false')
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
    "id"
  ],
  "decodes": [
    "Text"
  ],
  "client_limit": null
}
*/
