SELECT
  json_extract(CAST(r.data AS TEXT), '$.id') AS "id"
FROM resources r
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
  AND (EXISTS (SELECT 1 FROM json_each(CASE WHEN json_type(CAST(r.data AS TEXT), '$.contact') = 'array' THEN json_extract(CAST(r.data AS TEXT), '$.contact') WHEN json_type(CAST(r.data AS TEXT), '$.contact') IN ('object', 'array') THEN json_array(json(json_extract(CAST(r.data AS TEXT), '$.contact'))) WHEN json_type(CAST(r.data AS TEXT), '$.contact') IS NOT NULL THEN json_array(json_extract(CAST(r.data AS TEXT), '$.contact')) ELSE '[]' END) w0_0, json_each(CASE WHEN json_type(w0_0.value, '$.telecom') = 'array' THEN json_extract(w0_0.value, '$.telecom') WHEN json_type(w0_0.value, '$.telecom') IN ('object', 'array') THEN json_array(json(json_extract(w0_0.value, '$.telecom'))) WHEN json_type(w0_0.value, '$.telecom') IS NOT NULL THEN json_array(json_extract(w0_0.value, '$.telecom')) ELSE '[]' END) w0 WHERE ((json_extract(w0.value, '$.system') = 'phone') AND (json_extract(w0.value, '$.value') = '555')))) IS NOT NULL AND (EXISTS (SELECT 1 FROM json_each(CASE WHEN json_type(CAST(r.data AS TEXT), '$.contact') = 'array' THEN json_extract(CAST(r.data AS TEXT), '$.contact') WHEN json_type(CAST(r.data AS TEXT), '$.contact') IN ('object', 'array') THEN json_array(json(json_extract(CAST(r.data AS TEXT), '$.contact'))) WHEN json_type(CAST(r.data AS TEXT), '$.contact') IS NOT NULL THEN json_array(json_extract(CAST(r.data AS TEXT), '$.contact')) ELSE '[]' END) w0_0, json_each(CASE WHEN json_type(w0_0.value, '$.telecom') = 'array' THEN json_extract(w0_0.value, '$.telecom') WHEN json_type(w0_0.value, '$.telecom') IN ('object', 'array') THEN json_array(json(json_extract(w0_0.value, '$.telecom'))) WHEN json_type(w0_0.value, '$.telecom') IS NOT NULL THEN json_array(json_extract(w0_0.value, '$.telecom')) ELSE '[]' END) w0 WHERE ((json_extract(w0.value, '$.system') = 'phone') AND (json_extract(w0.value, '$.value') = '555')))) != 0 AND (EXISTS (SELECT 1 FROM json_each(CASE WHEN json_type(CAST(r.data AS TEXT), '$.contact') = 'array' THEN json_extract(CAST(r.data AS TEXT), '$.contact') WHEN json_type(CAST(r.data AS TEXT), '$.contact') IN ('object', 'array') THEN json_array(json(json_extract(CAST(r.data AS TEXT), '$.contact'))) WHEN json_type(CAST(r.data AS TEXT), '$.contact') IS NOT NULL THEN json_array(json_extract(CAST(r.data AS TEXT), '$.contact')) ELSE '[]' END) w0_0, json_each(CASE WHEN json_type(w0_0.value, '$.telecom') = 'array' THEN json_extract(w0_0.value, '$.telecom') WHEN json_type(w0_0.value, '$.telecom') IN ('object', 'array') THEN json_array(json(json_extract(w0_0.value, '$.telecom'))) WHEN json_type(w0_0.value, '$.telecom') IS NOT NULL THEN json_array(json_extract(w0_0.value, '$.telecom')) ELSE '[]' END) w0 WHERE ((json_extract(w0.value, '$.system') = 'phone') AND (json_extract(w0.value, '$.value') = '555')))) != 'false'
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
