SELECT
  CAST((SELECT json_group_array(ca0.value ORDER BY COALESCE(ca0.rowid, -1)) FROM json_each(CASE WHEN json_type(fe.value, '$.telecom') = 'array' THEN json_extract(fe.value, '$.telecom') WHEN json_type(fe.value, '$.telecom') IN ('object', 'array') THEN json_array(json(json_extract(fe.value, '$.telecom'))) WHEN json_type(fe.value, '$.telecom') IS NOT NULL THEN json_array(json_extract(fe.value, '$.telecom')) ELSE '[]' END) ca0) AS TEXT) AS "objects",
  CAST((SELECT json_group_array(ca1.value ORDER BY COALESCE(ca0.rowid, -1), COALESCE(ca1.rowid, -1)) FROM json_each(CASE WHEN json_type(fe.value, '$.telecom') = 'array' THEN json_extract(fe.value, '$.telecom') WHEN json_type(fe.value, '$.telecom') IN ('object', 'array') THEN json_array(json(json_extract(fe.value, '$.telecom'))) WHEN json_type(fe.value, '$.telecom') IS NOT NULL THEN json_array(json_extract(fe.value, '$.telecom')) ELSE '[]' END) ca0, json_each(CASE WHEN json_type(ca0.value, '$.value') = 'array' THEN json_extract(ca0.value, '$.value') ELSE json_array(json_extract(ca0.value, '$.value')) END) ca1 WHERE json_type(ca0.value, '$.value') IS NOT NULL) AS TEXT) AS "values",
  CAST((SELECT group_concat(ja1.value, '|' ORDER BY COALESCE(ja0.rowid, -1), COALESCE(ja1.rowid, -1)) FROM json_each(CASE WHEN json_type(fe.value, '$.telecom') = 'array' THEN json_extract(fe.value, '$.telecom') WHEN json_type(fe.value, '$.telecom') IN ('object', 'array') THEN json_array(json(json_extract(fe.value, '$.telecom'))) WHEN json_type(fe.value, '$.telecom') IS NOT NULL THEN json_array(json_extract(fe.value, '$.telecom')) ELSE '[]' END) ja0, json_each(CASE WHEN json_type(ja0.value, '$.value') = 'array' THEN json_extract(ja0.value, '$.value') WHEN json_type(ja0.value, '$.value') IN ('object', 'array') THEN json_array(json(json_extract(ja0.value, '$.value'))) WHEN json_type(ja0.value, '$.value') IS NOT NULL THEN json_array(json_extract(ja0.value, '$.value')) ELSE '[]' END) ja1) AS TEXT) AS "joined",
  CAST((SELECT json_extract(w2.value, '$.value') FROM json_each(CASE WHEN json_type(fe.value, '$.telecom') = 'array' THEN json_extract(fe.value, '$.telecom') WHEN json_type(fe.value, '$.telecom') IN ('object', 'array') THEN json_array(json(json_extract(fe.value, '$.telecom'))) WHEN json_type(fe.value, '$.telecom') IS NOT NULL THEN json_array(json_extract(fe.value, '$.telecom')) ELSE '[]' END) w2 WHERE (json_extract(w2.value, '$.system') = 'phone') ORDER BY COALESCE(w2.rowid, -1) LIMIT 1) AS TEXT) AS "pick"
FROM resources r
JOIN json_each(r.data, '$.contact') fe ON 1=1
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
      "value": "Patient"
    }
  ],
  "columns": [
    "objects",
    "values",
    "joined",
    "pick"
  ],
  "decodes": [
    "Json",
    "Json",
    "Text",
    "Text"
  ],
  "client_limit": null
}
*/
